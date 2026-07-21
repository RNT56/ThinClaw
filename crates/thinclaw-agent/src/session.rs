//! Session and thread model for turn-based agent interactions.
//!
//! A Session contains one or more Threads. Each Thread represents a
//! conversation/interaction sequence with the agent. Threads contain
//! Turns, which are request/response pairs.
//!
//! This model supports:
//! - Undo: Roll back to a previous turn
//! - Interrupt: Cancel the current turn mid-execution
//! - Compaction: Summarize old turns to save context
//! - Resume: Continue from a saved checkpoint

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use thinclaw_identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
use thinclaw_llm_core::{ChatMessage, Role, ToolCall};

use crate::personality::SessionPersonalityOverlay;
use crate::ports::{
    PortablePendingApproval, PortablePendingAuth, PortablePendingAuthMode, PortableThreadState,
    ThreadMessage, ThreadRuntimeSnapshot,
};

pub fn message_hides_user_input_in_main_chat(metadata: &serde_json::Value) -> bool {
    metadata
        .get("hide_user_input_from_webui_chat")
        .and_then(|value| value.as_bool())
        .or_else(|| {
            metadata
                .get("hide_from_webui_chat")
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(false)
}

pub fn message_is_startup_hook(metadata: &serde_json::Value) -> bool {
    metadata
        .get("synthetic_origin")
        .and_then(|value| value.as_str())
        == Some("startup_hook")
}

/// Whether a durable user row was intentionally injected as context without
/// starting an agent turn. This distinguishes a completed user-only context
/// entry from an incomplete hidden turn left behind by a crash.
pub fn message_is_context_only(metadata: &serde_json::Value) -> bool {
    metadata
        .get("thinclaw_context_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Message-metadata keys owned by the context pipeline. Channel/user metadata
/// must never be allowed to pre-populate these fields: hydration treats them
/// as durable records of a transformation that ThinClaw itself applied.
pub const EFFECTIVE_USER_INSTRUCTION_METADATA_KEY: &str = "_thinclaw_effective_user_instruction";
pub const EFFECTIVE_USER_INSTRUCTION_VERSION_METADATA_KEY: &str =
    "_thinclaw_effective_user_instruction_version";
pub const EFFECTIVE_USER_INSTRUCTION_VERSION: u64 = 1;
pub const MAX_EFFECTIVE_USER_INSTRUCTION_BYTES: usize = 100_000;

/// Return the model-visible user instruction recorded by ThinClaw, if any.
/// The raw conversation row remains unchanged for user-facing audit/history;
/// this typed metadata is what makes the transformed prompt replayable after
/// restart without granting ingress metadata the ability to spoof it.
pub fn effective_user_instruction(metadata: &serde_json::Value) -> Option<&str> {
    (metadata
        .get(EFFECTIVE_USER_INSTRUCTION_VERSION_METADATA_KEY)
        .and_then(serde_json::Value::as_u64)
        == Some(EFFECTIVE_USER_INSTRUCTION_VERSION))
    .then(|| {
        metadata
            .get(EFFECTIVE_USER_INSTRUCTION_METADATA_KEY)
            .and_then(serde_json::Value::as_str)
    })
    .flatten()
    .filter(|instruction| {
        !instruction.is_empty() && instruction.len() <= MAX_EFFECTIVE_USER_INSTRUCTION_BYTES
    })
}

fn default_true() -> bool {
    true
}

/// A session containing one or more threads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session ID.
    pub id: Uuid,
    /// User ID that owns this session.
    pub user_id: String,
    /// Principal ID for shared household ownership.
    pub principal_id: String,
    /// Actor ID owning this conversation scope.
    pub actor_id: String,
    /// Stable conversation scope ID.
    pub conversation_scope_id: Uuid,
    /// Direct/group conversation classification.
    pub conversation_kind: ConversationKind,
    /// Active thread ID.
    pub active_thread: Option<Uuid>,
    /// All threads in this session.
    pub threads: HashMap<Uuid, Thread>,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session was last active.
    pub last_active_at: DateTime<Utc>,
    /// Session metadata.
    pub metadata: serde_json::Value,
    /// Tools that have been auto-approved for this session ("always approve").
    #[serde(default)]
    pub auto_approved_tools: HashSet<String>,
    /// Temporary session-level personality overlay. This is intentionally not persisted.
    #[serde(skip)]
    pub active_personality: Option<SessionPersonalityOverlay>,
}

impl Session {
    /// Create a new session.
    pub fn new(user_id: impl Into<String>) -> Self {
        let user_id = user_id.into();
        let scope_id = scope_id_from_key(&format!("principal:{user_id}"));
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id: user_id.clone(),
            principal_id: user_id.clone(),
            actor_id: user_id,
            conversation_scope_id: scope_id,
            conversation_kind: ConversationKind::Direct,
            active_thread: None,
            threads: HashMap::new(),
            created_at: now,
            last_active_at: now,
            metadata: serde_json::Value::Null,
            auto_approved_tools: HashSet::new(),
            active_personality: None,
        }
    }

    /// Create a session with explicit principal/actor identity.
    pub fn new_scoped(
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
        conversation_scope_id: Uuid,
        conversation_kind: ConversationKind,
    ) -> Self {
        let principal_id = principal_id.into();
        let actor_id = actor_id.into();
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id: principal_id.clone(),
            principal_id,
            actor_id,
            conversation_scope_id,
            conversation_kind,
            active_thread: None,
            threads: HashMap::new(),
            created_at: now,
            last_active_at: now,
            metadata: serde_json::Value::Null,
            auto_approved_tools: HashSet::new(),
            active_personality: None,
        }
    }

    /// Create a session directly from a resolved identity.
    pub fn from_identity(identity: &ResolvedIdentity) -> Self {
        Self::new_scoped(
            identity.principal_id.clone(),
            identity.actor_id.clone(),
            identity.conversation_scope_id,
            identity.conversation_kind,
        )
    }

    /// Check if a tool has been auto-approved for this session.
    pub fn is_tool_auto_approved(&self, tool_name: &str) -> bool {
        self.auto_approved_tools.contains(tool_name)
    }

    /// Check if a tool has been auto-approved for the given channel.
    ///
    /// Legacy global approvals (`tool_name`) remain valid for backward
    /// compatibility. New approvals are stored in channel-scoped form.
    pub fn is_tool_auto_approved_for_channel(&self, channel: &str, tool_name: &str) -> bool {
        self.is_tool_auto_approved(tool_name)
            || self
                .auto_approved_tools
                .contains(&Self::channel_tool_approval_key(channel, tool_name))
    }

    /// Add a tool to the auto-approved set.
    pub fn auto_approve_tool(&mut self, tool_name: impl Into<String>) {
        self.auto_approved_tools.insert(tool_name.into());
    }

    /// Add a channel-scoped tool approval.
    pub fn auto_approve_tool_for_channel(&mut self, channel: &str, tool_name: &str) {
        self.auto_approved_tools
            .insert(Self::channel_tool_approval_key(channel, tool_name));
    }

    /// Check an actor-scoped session approval. Direct sessions are already
    /// actor-isolated, so legacy global/channel keys remain valid there.
    /// Group sessions deliberately ignore legacy shared keys because applying
    /// them would let one participant grant permissions to every participant.
    pub fn is_tool_auto_approved_for_identity(
        &self,
        actor_id: &str,
        channel: &str,
        tool_name: &str,
    ) -> bool {
        self.auto_approved_tools
            .contains(&Self::actor_channel_tool_approval_key(
                actor_id, channel, tool_name,
            ))
            || (self.conversation_kind == ConversationKind::Direct
                && self.actor_id == actor_id
                && self.is_tool_auto_approved_for_channel(channel, tool_name))
    }

    /// Add an approval that belongs only to the actor who granted it.
    pub fn auto_approve_tool_for_identity(
        &mut self,
        actor_id: &str,
        channel: &str,
        tool_name: &str,
    ) {
        self.auto_approved_tools
            .insert(Self::actor_channel_tool_approval_key(
                actor_id, channel, tool_name,
            ));
    }

    fn channel_tool_approval_key(channel: &str, tool_name: &str) -> String {
        format!("channel:{channel}:tool:{tool_name}")
    }

    fn actor_channel_tool_approval_key(actor_id: &str, channel: &str, tool_name: &str) -> String {
        format!(
            "actor:{}:{actor_id}:channel:{}:{channel}:tool:{}:{tool_name}",
            actor_id.len(),
            channel.len(),
            tool_name.len()
        )
    }

    /// Create a new thread in this session.
    pub fn create_thread(&mut self) -> &mut Thread {
        self.insert_thread(Thread::new(self.id))
    }

    /// Insert an already-created thread into this session and activate it.
    pub fn insert_thread(&mut self, thread: Thread) -> &mut Thread {
        let thread_id = thread.id;
        self.active_thread = Some(thread_id);
        self.last_active_at = Utc::now();
        self.threads.entry(thread_id).or_insert(thread)
    }

    /// Get the active thread.
    pub fn active_thread(&self) -> Option<&Thread> {
        self.active_thread.and_then(|id| self.threads.get(&id))
    }

    /// Get the active thread mutably.
    pub fn active_thread_mut(&mut self) -> Option<&mut Thread> {
        self.active_thread.and_then(|id| self.threads.get_mut(&id))
    }

    /// Get or create the active thread.
    pub fn get_or_create_thread(&mut self) -> &mut Thread {
        match self.active_thread {
            None => self.create_thread(),
            Some(id) => {
                let session_id = self.id;
                // Recover a stale active-thread pointer without taking a
                // second mutable borrow of `self`. Reusing the durable thread
                // ID also keeps session/conversation identity stable.
                self.threads
                    .entry(id)
                    .or_insert_with(|| Thread::with_id(id, session_id))
            }
        }
    }

    /// Switch to a different thread.
    pub fn switch_thread(&mut self, thread_id: Uuid) -> bool {
        if self.threads.contains_key(&thread_id) {
            self.active_thread = Some(thread_id);
            self.last_active_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Mark this session as active right now.
    ///
    /// Thread creation, switching, and hydration already update
    /// `last_active_at`, but those only happen at the start/edges of a
    /// conversation. Call this wherever a turn is actually recorded (e.g.
    /// alongside runtime-snapshot persistence) so a long-running
    /// conversation is not pruned as idle by `prune_stale_sessions` while
    /// still mid-turn.
    pub fn touch_last_active(&mut self) {
        self.last_active_at = Utc::now();
    }
}

/// State of a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThreadState {
    /// Thread is idle, waiting for input.
    #[default]
    Idle,
    /// Thread is processing a turn.
    Processing,
    /// Thread is waiting for user approval.
    AwaitingApproval,
    /// Thread has completed (no more turns expected).
    Completed,
    /// Thread was interrupted.
    Interrupted,
}

/// Pending auth token request.
///
/// When `tool_auth` returns `awaiting_token`, the thread enters auth mode.
/// The next user message is intercepted before entering the normal pipeline
/// (no logging, no turn creation, no history) and routed directly to the
/// credential store.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingAuthMode {
    ManualToken,
    ExternalOAuth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingAuth {
    /// Extension name to authenticate.
    pub extension_name: String,
    #[serde(default = "default_pending_auth_mode")]
    pub auth_mode: PendingAuthMode,
    /// Actor that initiated this authentication request. Missing identities
    /// are legacy state and are deliberately not accepted for token capture.
    #[serde(default)]
    pub requesting_identity: Option<ResolvedIdentity>,
}

fn default_pending_auth_mode() -> PendingAuthMode {
    PendingAuthMode::ManualToken
}

/// Pending tool approval request stored on a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    /// Unique request ID.
    pub request_id: Uuid,
    /// Tool name requiring approval.
    pub tool_name: String,
    /// Tool parameters.
    pub parameters: serde_json::Value,
    /// Description of what the tool will do.
    pub description: String,
    /// Tool call ID from LLM (for proper context continuation).
    pub tool_call_id: String,
    /// Context messages at the time of the request (to resume from).
    pub context_messages: Vec<ChatMessage>,
    /// Remaining tool calls from the same assistant message that were not
    /// executed yet when approval was requested.
    #[serde(default)]
    pub deferred_tool_calls: Vec<ToolCall>,
    /// Actor and ingress context that initiated this request.
    #[serde(default)]
    pub requesting_identity: Option<ResolvedIdentity>,
    #[serde(default)]
    pub request_channel: String,
    #[serde(default)]
    pub request_metadata: serde_json::Value,
}

fn same_requesting_actor(expected: &ResolvedIdentity, actual: &ResolvedIdentity) -> bool {
    expected.principal_id == actual.principal_id
        && expected.actor_id == actual.actor_id
        && expected.conversation_kind == actual.conversation_kind
        && (expected.conversation_kind == ConversationKind::Direct
            || expected.conversation_scope_id == actual.conversation_scope_id)
}

impl PendingAuth {
    /// Whether this credential-bearing response belongs to the actor that
    /// initiated the flow. Legacy unbound state intentionally matches nobody.
    pub fn accepts_identity(&self, identity: &ResolvedIdentity) -> bool {
        self.requesting_identity
            .as_ref()
            .is_some_and(|expected| same_requesting_actor(expected, identity))
    }
}

impl PendingApproval {
    /// Whether an approval/rejection response is from the original requester.
    /// Legacy unbound state intentionally matches nobody.
    pub fn accepts_identity(&self, identity: &ResolvedIdentity) -> bool {
        self.requesting_identity
            .as_ref()
            .is_some_and(|expected| same_requesting_actor(expected, identity))
    }
}

impl From<ThreadState> for PortableThreadState {
    fn from(value: ThreadState) -> Self {
        match value {
            ThreadState::Idle => Self::Idle,
            ThreadState::Processing => Self::Processing,
            ThreadState::AwaitingApproval => Self::AwaitingApproval,
            ThreadState::Completed => Self::Completed,
            ThreadState::Interrupted => Self::Interrupted,
        }
    }
}

impl From<PortableThreadState> for ThreadState {
    fn from(value: PortableThreadState) -> Self {
        match value {
            PortableThreadState::Idle => Self::Idle,
            PortableThreadState::Processing => Self::Processing,
            PortableThreadState::AwaitingApproval => Self::AwaitingApproval,
            PortableThreadState::Completed => Self::Completed,
            PortableThreadState::Interrupted => Self::Interrupted,
        }
    }
}

impl From<PendingAuthMode> for PortablePendingAuthMode {
    fn from(value: PendingAuthMode) -> Self {
        match value {
            PendingAuthMode::ManualToken => Self::ManualToken,
            PendingAuthMode::ExternalOAuth => Self::ExternalOAuth,
        }
    }
}

impl From<PortablePendingAuthMode> for PendingAuthMode {
    fn from(value: PortablePendingAuthMode) -> Self {
        match value {
            PortablePendingAuthMode::ManualToken => Self::ManualToken,
            PortablePendingAuthMode::ExternalOAuth => Self::ExternalOAuth,
        }
    }
}

impl From<PendingAuth> for PortablePendingAuth {
    fn from(value: PendingAuth) -> Self {
        Self {
            extension_name: value.extension_name,
            auth_mode: value.auth_mode.into(),
            requesting_identity: value.requesting_identity,
        }
    }
}

impl From<PortablePendingAuth> for PendingAuth {
    fn from(value: PortablePendingAuth) -> Self {
        Self {
            extension_name: value.extension_name,
            auth_mode: value.auth_mode.into(),
            requesting_identity: value.requesting_identity,
        }
    }
}

impl From<PendingApproval> for PortablePendingApproval {
    fn from(value: PendingApproval) -> Self {
        Self {
            request_id: value.request_id,
            tool_name: value.tool_name,
            parameters: value.parameters,
            description: value.description,
            tool_call_id: value.tool_call_id,
            context_messages: value.context_messages,
            deferred_tool_calls: value.deferred_tool_calls,
            requesting_identity: value.requesting_identity,
            request_channel: value.request_channel,
            request_metadata: value.request_metadata,
        }
    }
}

impl From<PortablePendingApproval> for PendingApproval {
    fn from(value: PortablePendingApproval) -> Self {
        Self {
            request_id: value.request_id,
            tool_name: value.tool_name,
            parameters: value.parameters,
            description: value.description,
            tool_call_id: value.tool_call_id,
            context_messages: value.context_messages,
            deferred_tool_calls: value.deferred_tool_calls,
            requesting_identity: value.requesting_identity,
            request_channel: value.request_channel,
            request_metadata: value.request_metadata,
        }
    }
}

/// A conversation thread within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    /// Unique thread ID.
    pub id: Uuid,
    /// Parent session ID.
    pub session_id: Uuid,
    /// Current state.
    pub state: ThreadState,
    /// Turns in this thread.
    pub turns: Vec<Turn>,
    /// When the thread was created.
    pub created_at: DateTime<Utc>,
    /// When the thread was last updated.
    pub updated_at: DateTime<Utc>,
    /// Thread metadata (e.g., title, tags).
    pub metadata: serde_json::Value,
    /// Pending approval request (when state is AwaitingApproval).
    #[serde(default)]
    pub pending_approval: Option<PendingApproval>,
    /// Pending auth token request (thread is in auth mode).
    #[serde(default)]
    pub pending_auth: Option<PendingAuth>,
    /// Plan mode: when true, mutating (non-read) tools are hidden from the LLM's
    /// tool list and, if attempted, require operator approval before running —
    /// so the agent proposes actions and the operator confirms. Toggled by
    /// `/plan`; survives restart via the runtime snapshot.
    #[serde(default)]
    pub plan_mode: bool,
}

impl Thread {
    /// Create a new thread.
    pub fn new(session_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            session_id,
            state: ThreadState::Idle,
            turns: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Null,
            pending_approval: None,
            pending_auth: None,
            plan_mode: false,
        }
    }

    /// Create a thread with a specific ID (for DB hydration).
    pub fn with_id(id: Uuid, session_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id,
            session_id,
            state: ThreadState::Idle,
            turns: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Null,
            pending_approval: None,
            pending_auth: None,
            plan_mode: false,
        }
    }

    pub fn runtime_snapshot(
        &self,
        owner_agent_id: Option<String>,
        model_override: Option<crate::ports::ModelOverride>,
        auto_approved_tools: impl IntoIterator<Item = String>,
        active_subagents: Vec<crate::ports::PortableSubagentState>,
        last_context_pressure: Option<serde_json::Value>,
    ) -> ThreadRuntimeSnapshot {
        let mut auto_approved_tools: Vec<String> = auto_approved_tools.into_iter().collect();
        auto_approved_tools.sort();
        auto_approved_tools.dedup();
        ThreadRuntimeSnapshot {
            state: self.state.into(),
            pending_approval: self.pending_approval.clone().map(Into::into),
            pending_auth: self.pending_auth.clone().map(Into::into),
            owner_agent_id,
            model_override,
            auto_approved_tools,
            active_subagents,
            last_context_pressure,
            post_compaction_context: None,
            frozen_workspace_prompt: None,
            frozen_provider_system_prompt: None,
            prompt_snapshot_hash: None,
            ephemeral_overlay_hash: None,
            prompt_contract_version: None,
            prompt_manifest_digest: None,
            prompt_segment_order: Vec::new(),
            provider_context_refs: Vec::new(),
            active_message_start_row: None,
            active_message_row_count: None,
            inflight_tool_trace: self
                .last_turn()
                .filter(|turn| turn.response.is_none())
                .map(|turn| durable_tool_trace(&turn.tool_calls))
                .unwrap_or_default(),
            undo_checkpoints: Vec::new(),
            plan_mode: self.plan_mode,
        }
    }

    pub fn restore_runtime_snapshot(&mut self, runtime: ThreadRuntimeSnapshot) {
        let restored_state = ThreadState::from(runtime.state);
        let inflight_tool_trace = runtime.inflight_tool_trace.clone();
        self.pending_approval = runtime
            .pending_approval
            .map(PendingApproval::from)
            .filter(|pending| pending.requesting_identity.is_some());
        self.pending_auth = runtime
            .pending_auth
            .map(PendingAuth::from)
            .filter(|pending| pending.requesting_identity.is_some());
        self.plan_mode = runtime.plan_mode;

        let pending_turn_is_resumable = self
            .pending_approval
            .clone()
            .is_some_and(|pending| self.restore_pending_approval_turn(&pending));
        if self.pending_approval.is_some() && !pending_turn_is_resumable {
            tracing::warn!(
                thread = %self.id,
                "Discarding persisted approval without a resumable user turn"
            );
            self.pending_approval = None;
        }

        if !inflight_tool_trace.is_empty()
            && let Some(turn) = self.turns.last_mut().filter(|turn| turn.response.is_none())
        {
            for persisted in inflight_tool_trace {
                if let Some(existing) = turn
                    .tool_calls
                    .iter_mut()
                    .find(|existing| existing.id == persisted.id)
                {
                    if persisted.result.is_some() || persisted.error.is_some() {
                        existing.result = persisted.result;
                        existing.error = persisted.error;
                    }
                } else {
                    turn.tool_calls.push(persisted);
                }
            }
        }

        self.state = if self.pending_approval.is_some() {
            ThreadState::AwaitingApproval
        } else {
            match restored_state {
                // A process cannot safely resume an arbitrary provider/tool
                // future. A waiting-approval state without a valid pending
                // envelope is equally non-resumable and must not strand the
                // thread in a state that no response can satisfy.
                ThreadState::Processing | ThreadState::AwaitingApproval => {
                    if let Some(turn) = self
                        .turns
                        .iter_mut()
                        .rev()
                        .find(|turn| turn.response.is_none())
                    {
                        turn.interrupt();
                    }
                    ThreadState::Interrupted
                }
                other => other,
            }
        };
        self.updated_at = Utc::now();
    }

    /// Rebuild the live tool audit state needed to continue a persisted
    /// approval after restart. Durable conversation rows contain the user
    /// message but the assistant tool-call block is held in the pending
    /// context snapshot until the final assistant response is written.
    fn restore_pending_approval_turn(&mut self, pending: &PendingApproval) -> bool {
        let Some(user_input) = pending
            .context_messages
            .iter()
            .rev()
            .find(|message| message.is_user_instruction())
            .map(|message| message.content.clone())
        else {
            return false;
        };
        if self
            .turns
            .last()
            .is_none_or(|turn| turn.response.is_some() || turn.user_input != user_input)
        {
            self.turns
                .push(Turn::new(self.turns.len(), user_input, false));
        }

        let Some(turn) = self.turns.last_mut() else {
            return false;
        };
        if turn.response.is_some() {
            return false;
        }
        turn.state = TurnState::Processing;
        turn.completed_at = None;
        turn.error = None;

        let tool_block = pending
            .context_messages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, message)| {
                message.role == Role::Assistant
                    && message
                        .tool_calls
                        .as_ref()
                        .is_some_and(|calls| !calls.is_empty())
            });

        if let Some((assistant_index, assistant)) = tool_block {
            for call in assistant.tool_calls.clone().unwrap_or_default() {
                if !turn
                    .tool_calls
                    .iter()
                    .any(|existing| existing.id == call.id)
                {
                    turn.record_tool_call_with_id(call.id, call.name, call.arguments);
                }
            }
            for result in pending.context_messages.iter().skip(assistant_index + 1) {
                if result.role != Role::Tool {
                    continue;
                }
                let Some(call_id) = result.tool_call_id.as_deref() else {
                    continue;
                };
                if let Some(error) = result.content.strip_prefix("[error] ") {
                    turn.record_tool_error_for_id(call_id, error.to_string());
                } else {
                    turn.record_tool_result_for_id(
                        call_id,
                        serde_json::Value::String(result.content.clone()),
                    );
                }
            }
        }

        // Defensive fallback for legacy/trimmed context snapshots: the
        // current pending call and its deferred siblings are sufficient to
        // preserve provider-valid ordering for continuation.
        if !turn
            .tool_calls
            .iter()
            .any(|call| call.id == pending.tool_call_id)
        {
            turn.record_tool_call_with_id(
                pending.tool_call_id.clone(),
                pending.tool_name.clone(),
                pending.parameters.clone(),
            );
        }
        for call in &pending.deferred_tool_calls {
            if !turn
                .tool_calls
                .iter()
                .any(|existing| existing.id == call.id)
            {
                turn.record_tool_call_with_id(
                    call.id.clone(),
                    call.name.clone(),
                    call.arguments.clone(),
                );
            }
        }
        true
    }

    /// Get the current turn number (1-indexed for display).
    pub fn turn_number(&self) -> usize {
        self.turns.len() + 1
    }

    /// Get the last turn.
    pub fn last_turn(&self) -> Option<&Turn> {
        self.turns.last()
    }

    /// Get the last turn mutably.
    pub fn last_turn_mut(&mut self) -> Option<&mut Turn> {
        self.turns.last_mut()
    }

    /// Start a new turn with user input.
    pub fn start_turn(&mut self, user_input: impl Into<String>) -> &mut Turn {
        self.start_turn_with_visibility(user_input, false)
    }

    /// Start a new turn with user input and explicit user-message visibility.
    pub fn start_turn_with_visibility(
        &mut self,
        user_input: impl Into<String>,
        hide_user_input_from_ui: bool,
    ) -> &mut Turn {
        let turn_number = self.turns.len();
        let turn = Turn::new(turn_number, user_input, hide_user_input_from_ui);
        self.turns.push(turn);
        self.state = ThreadState::Processing;
        self.updated_at = Utc::now();
        // turn_number was len() before push, so it's a valid index after push
        &mut self.turns[turn_number]
    }

    /// Complete the current turn with a response.
    pub fn complete_turn(&mut self, response: impl Into<String>) {
        if let Some(turn) = self
            .turns
            .iter_mut()
            .rev()
            .find(|turn| turn.state == TurnState::Processing)
        {
            turn.complete(response);
            self.state = ThreadState::Idle;
            self.updated_at = Utc::now();
        }
    }

    /// Fail the current turn with an error.
    pub fn fail_turn(&mut self, error: impl Into<String>) {
        if let Some(turn) = self
            .turns
            .iter_mut()
            .rev()
            .find(|turn| turn.state == TurnState::Processing)
        {
            turn.fail(error);
            self.state = ThreadState::Idle;
            self.updated_at = Utc::now();
        }
    }

    /// Mark the thread as awaiting approval with pending request details.
    pub fn await_approval(&mut self, pending: PendingApproval) {
        self.state = ThreadState::AwaitingApproval;
        self.pending_approval = Some(pending);
        self.updated_at = Utc::now();
    }

    /// Take the pending approval (clearing it from the thread).
    pub fn take_pending_approval(&mut self) -> Option<PendingApproval> {
        self.pending_approval.take()
    }

    /// Clear pending approval and return to idle state.
    pub fn clear_pending_approval(&mut self) {
        self.pending_approval = None;
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Enter auth mode: next user message will be routed directly to
    /// the credential store, bypassing the normal pipeline entirely.
    pub fn enter_auth_mode(
        &mut self,
        extension_name: String,
        auth_mode: PendingAuthMode,
        requesting_identity: ResolvedIdentity,
    ) {
        self.pending_auth = Some(PendingAuth {
            extension_name,
            auth_mode,
            requesting_identity: Some(requesting_identity),
        });
        self.updated_at = Utc::now();
    }

    /// Take the pending auth (clearing auth mode).
    pub fn take_pending_auth(&mut self) -> Option<PendingAuth> {
        self.pending_auth.take()
    }

    /// Interrupt the current turn.
    pub fn interrupt(&mut self) {
        if let Some(turn) = self
            .turns
            .iter_mut()
            .rev()
            .find(|turn| turn.state == TurnState::Processing)
        {
            turn.interrupt();
        }
        self.pending_approval = None;
        self.pending_auth = None;
        self.state = ThreadState::Interrupted;
        self.updated_at = Utc::now();
    }

    /// Resume after interruption.
    pub fn resume(&mut self) {
        if self.state == ThreadState::Interrupted {
            self.state = ThreadState::Idle;
            self.updated_at = Utc::now();
        }
    }

    /// Append trusted context without starting an LLM turn. The entry is a
    /// completed user-only turn, so it is visible to subsequent prompts while
    /// never becoming the target of a later active-turn completion.
    pub fn inject_context(&mut self, content: impl Into<String>, hide_from_ui: bool) {
        let mut turn = Turn::new(self.turns.len(), content, hide_from_ui);
        turn.complete_without_response();
        self.turns.push(turn);
        self.updated_at = Utc::now();
    }

    /// Get all messages for context building.
    ///
    /// Reconstructs the full conversation, including prior turns' tool calls and
    /// their results, so a later turn can "see" exactly what an earlier turn's
    /// tools returned (e.g. a grep or test-run output) rather than only the
    /// final prose. Each reconstructed turn emits a provider-valid shape:
    ///
    /// ```text
    /// user(input)
    /// assistant(tool_calls: [t{n}_c0, t{n}_c1, ...])   // if the turn used tools
    /// tool_result(t{n}_c0) ... tool_result(t{n}_cK)    // one per call, in order
    /// assistant(response)                              // final text, if present
    /// ```
    ///
    /// Every reconstructed `tool_call` id is paired with exactly one following
    /// `tool_result` (a placeholder when no result/error was recorded), so the
    /// sequence never leaves an orphaned tool call for a provider to reject.
    /// Individual tool-result bodies are truncated to bound context growth from
    /// a single very large historical output; the live agent loop applies its
    /// own additional token cap and recent-turns pruning downstream.
    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();
        for turn in &self.turns {
            let mut user_message = ChatMessage::user(&turn.user_input);
            if !turn.has_durable_user_row {
                user_message = user_message.with_provider_metadata(
                    "thinclaw_turn_state",
                    serde_json::json!({"has_durable_user_row": false}),
                );
            }
            messages.push(user_message);
            messages.extend(turn.untrusted_contexts.iter().map(|context| {
                ChatMessage::untrusted_context(
                    &context.segment_id,
                    &context.source,
                    &context.content,
                )
            }));
            turn.append_tool_exchange(&mut messages);
            if let Some(ref response) = turn.response {
                messages.push(ChatMessage::assistant(response));
            }
        }
        messages
    }

    /// Number of durable conversation rows this thread represents: normally
    /// one `user` row per turn plus one `assistant` row per completed turn.
    /// Synthetic assistant-only startup turns explicitly contribute no user
    /// row.
    ///
    /// This is the unit the active-message watermark is stored in (it drives
    /// DB-row truncation on hydration after `/undo`, `/redo`, `/clear`,
    /// `/rewind`). It must **exclude** the synthetic tool-call/tool-result
    /// messages that [`Thread::messages`] reconstructs — those are never
    /// persisted as rows, so counting them would inflate the watermark and let
    /// undone turns reappear after a restart. Equivalent to what
    /// `messages().len()` returned before cross-turn tool reconstruction.
    pub fn persisted_message_count(&self) -> usize {
        self.turns
            .iter()
            .map(|turn| {
                usize::from(turn.has_durable_user_row) + usize::from(turn.response.is_some())
            })
            .sum()
    }

    /// Truncate turns to a specific count (keeping most recent).
    pub fn truncate_turns(&mut self, keep: usize) {
        if self.turns.len() > keep {
            let drain_count = self.turns.len() - keep;
            self.turns.drain(0..drain_count);
            // Re-number remaining turns
            for (i, turn) in self.turns.iter_mut().enumerate() {
                turn.turn_number = i;
            }
        }
    }

    /// Restore thread state from a checkpoint's messages.
    ///
    /// Clears existing turns and rebuilds from the message stream produced by
    /// [`Thread::messages`]. A turn is `user` followed by an optional tool
    /// exchange — `assistant(tool_calls)` + `tool_result(...)` — and an optional
    /// final `assistant(text)`. Tool calls/results are reconstructed onto the
    /// turn so a subsequent [`Thread::messages`] call reproduces an equivalent,
    /// provider-valid stream. Bare `user`/`assistant` alternation (no tools)
    /// still round-trips exactly as before.
    pub fn restore_from_messages(&mut self, messages: Vec<ChatMessage>) {
        self.turns.clear();
        self.state = ThreadState::Idle;
        self.pending_approval = None;
        self.pending_auth = None;

        let mut iter = messages.into_iter().peekable();
        let mut turn_number = 0;

        while let Some(msg) = iter.next() {
            if msg.role != Role::User {
                // Stray leading tool/assistant messages with no owning user turn
                // are skipped (matches prior behavior for malformed input).
                continue;
            }

            let has_durable_user_row = msg
                .provider_metadata
                .get("thinclaw_turn_state")
                .and_then(|metadata| metadata.get("has_durable_user_row"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            let mut turn = Turn::new(turn_number, &msg.content, false);
            turn.has_durable_user_row = has_durable_user_row;

            // Consume the rest of this turn: tool-call exchanges and/or a final
            // assistant text, stopping at the next user message.
            while let Some(next) = iter.peek() {
                match next.role {
                    Role::User => {
                        let Some((segment_id, source)) = next.untrusted_context_identity() else {
                            break;
                        };
                        let segment_id = segment_id.to_string();
                        let source = source.to_string();
                        let Some(evidence) = iter.next() else { break };
                        let content = evidence
                            .untrusted_context_raw_content()
                            .unwrap_or(&evidence.content)
                            .to_string();
                        turn.add_untrusted_context(segment_id, source, content);
                    }
                    Role::Assistant => {
                        // Guaranteed Some after a successful peek().
                        let Some(assistant) = iter.next() else { break };
                        if let Some(calls) = assistant.tool_calls {
                            // Tool-call block: register every call first, then
                            // attach each contiguous result by provider-issued
                            // ID. Parallel batches may finish out of order.
                            for call in calls {
                                turn.record_tool_call_with_id(call.id, call.name, call.arguments);
                            }
                            while iter.peek().is_some_and(|result| result.role == Role::Tool) {
                                let Some(result) = iter.next() else { break };
                                let Some(call_id) = result.tool_call_id.as_deref() else {
                                    continue;
                                };
                                if let Some(err) = result.content.strip_prefix("[error] ") {
                                    turn.record_tool_error_for_id(call_id, err.to_string());
                                } else {
                                    turn.record_tool_result_for_id(
                                        call_id,
                                        serde_json::Value::String(result.content),
                                    );
                                }
                            }
                        } else {
                            // Final assistant text for this turn.
                            turn.complete(&assistant.content);
                            break;
                        }
                    }
                    Role::Tool => {
                        // A tool result with no preceding recorded call: skip it
                        // rather than leave it dangling.
                        let _ = iter.next();
                    }
                    Role::System => {
                        let _ = iter.next();
                    }
                }
            }

            if turn.state == TurnState::Processing {
                turn.complete_without_response();
            }

            self.turns.push(turn);
            turn_number += 1;
        }

        self.updated_at = Utc::now();
    }

    /// Restore thread turns from durable conversation rows.
    ///
    /// Startup-hook assistant messages are preserved as hidden turns so a
    /// synthetic startup exchange still appears in the in-memory turn model.
    pub fn restore_from_thread_messages(&mut self, messages: &[ThreadMessage]) {
        self.turns.clear();
        self.state = ThreadState::Idle;
        self.pending_approval = None;
        self.pending_auth = None;

        let mut iter = messages.iter().peekable();
        let mut turn_number = 0;

        while let Some(message) = iter.next() {
            if message.role != "user" {
                if message.role == "assistant" && message_is_startup_hook(&message.metadata) {
                    let mut turn = Turn::new(turn_number, "", true);
                    turn.has_durable_user_row = false;
                    turn.complete(&message.content);
                    self.turns.push(turn);
                    turn_number += 1;
                }
                continue;
            }

            let hide_user_input_from_ui = message_hides_user_input_in_main_chat(&message.metadata);
            let replayed_user_input =
                effective_user_instruction(&message.metadata).unwrap_or(&message.content);
            let mut turn = Turn::new(turn_number, replayed_user_input, hide_user_input_from_ui);
            turn.durable_user_message_id = Some(message.id);
            if let Some(contexts) = message
                .metadata
                .get("untrusted_attachment_contexts")
                .and_then(|value| {
                    serde_json::from_value::<Vec<TurnContextEvidence>>(value.clone()).ok()
                })
            {
                turn.untrusted_contexts = bounded_turn_context_evidence(&contexts);
            }

            if let Some(next) = iter.peek()
                && next.role == "assistant"
                && let Some(response) = iter.next()
            {
                if let Some(trace) = response.metadata.get("tool_trace") {
                    match serde_json::from_value::<Vec<TurnToolCall>>(trace.clone()) {
                        Ok(tool_calls) => turn.tool_calls = durable_tool_trace(&tool_calls),
                        Err(error) => tracing::warn!(
                            message_id = %response.id,
                            error = %error,
                            "Ignoring invalid durable tool trace"
                        ),
                    }
                }
                turn.complete(&response.content);
            }

            if turn.hide_user_input_from_ui
                && turn.response.is_none()
                && !message_is_context_only(&message.metadata)
            {
                continue;
            }

            if turn.state == TurnState::Processing {
                if message_is_context_only(&message.metadata) {
                    turn.complete_without_response();
                } else {
                    // A durable user row without an assistant row is normally
                    // a turn interrupted by a process crash. Do not silently
                    // relabel it as a successfully completed context entry;
                    // pending approvals explicitly restore it to Processing
                    // from their runtime envelope below this hydration layer.
                    turn.interrupt();
                }
            }

            self.turns.push(turn);
            turn_number += 1;
        }

        self.updated_at = Utc::now();
    }
}

/// State of a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnState {
    /// Turn is being processed.
    Processing,
    /// Turn completed successfully.
    Completed,
    /// Turn failed with an error.
    Failed,
    /// Turn was interrupted.
    Interrupted,
}

/// A single turn (request/response pair) in a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    /// Turn number (0-indexed).
    pub turn_number: usize,
    /// User input that started this turn.
    pub user_input: String,
    /// Whether the user-side prompt should be hidden from the main WebUI chat transcript.
    #[serde(default, alias = "hidden_from_ui")]
    pub hide_user_input_from_ui: bool,
    /// Whether this turn corresponds to a durable user row. Synthetic
    /// assistant-only startup turns set this to false so active-history
    /// windows remain measured in actual database rows.
    #[serde(default = "default_true")]
    pub has_durable_user_row: bool,
    /// Exact durable user-row identifier for the live turn. Hydration restores
    /// it from the append-only transcript so post-ingress prompt transforms
    /// can update only their owning row. It is intentionally optional for
    /// deployments without a conversation store and legacy snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durable_user_message_id: Option<Uuid>,
    /// Agent response (if completed).
    pub response: Option<String>,
    /// Tool calls made during this turn.
    pub tool_calls: Vec<TurnToolCall>,
    /// Evidence extracted from user-supplied documents or other untrusted
    /// context sources. Kept separate from the user's instruction.
    #[serde(default)]
    pub untrusted_contexts: Vec<TurnContextEvidence>,
    /// Turn state.
    pub state: TurnState,
    /// When the turn started.
    pub started_at: DateTime<Utc>,
    /// When the turn completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message (if failed).
    pub error: Option<String>,
}

impl Turn {
    /// Create a new turn.
    pub fn new(
        turn_number: usize,
        user_input: impl Into<String>,
        hide_user_input_from_ui: bool,
    ) -> Self {
        Self {
            turn_number,
            user_input: user_input.into(),
            hide_user_input_from_ui,
            has_durable_user_row: true,
            durable_user_message_id: None,
            response: None,
            tool_calls: Vec::new(),
            untrusted_contexts: Vec::new(),
            state: TurnState::Processing,
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        }
    }

    /// Complete this turn.
    pub fn complete(&mut self, response: impl Into<String>) {
        self.response = Some(response.into());
        self.state = TurnState::Completed;
        self.completed_at = Some(Utc::now());
    }

    /// Mark a context-only user entry complete without synthesizing an empty
    /// assistant response.
    pub fn complete_without_response(&mut self) {
        self.response = None;
        self.state = TurnState::Completed;
        self.completed_at = Some(Utc::now());
    }

    /// Fail this turn.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.state = TurnState::Failed;
        self.completed_at = Some(Utc::now());
    }

    /// Interrupt this turn.
    pub fn interrupt(&mut self) {
        self.state = TurnState::Interrupted;
        self.completed_at = Some(Utc::now());
    }

    /// Record a tool call.
    pub fn record_tool_call(&mut self, name: impl Into<String>, params: serde_json::Value) {
        let id = synthetic_tool_call_id(self.turn_number, self.tool_calls.len());
        self.record_tool_call_with_id(id, name, params);
    }

    pub fn add_untrusted_context(
        &mut self,
        segment_id: impl Into<String>,
        source: impl Into<String>,
        content: impl Into<String>,
    ) {
        let candidate = TurnContextEvidence {
            segment_id: segment_id.into(),
            source: source.into(),
            content: content.into(),
        };
        let remaining =
            MAX_TURN_CONTEXT_EVIDENCE_ITEMS.saturating_sub(self.untrusted_contexts.len());
        if remaining == 0 {
            return;
        }
        let used_chars = self
            .untrusted_contexts
            .iter()
            .map(|context| context.content.chars().count())
            .sum::<usize>();
        let remaining_chars = MAX_TURN_CONTEXT_EVIDENCE_CHARS.saturating_sub(used_chars);
        if let Some(context) = bounded_context_evidence_item(&candidate, remaining_chars) {
            self.untrusted_contexts.push(context);
        }
    }

    /// Record a tool call using the provider-issued ID. Results must be
    /// attached through this ID because tool batches can execute concurrently
    /// and complete out of order.
    pub fn record_tool_call_with_id(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        params: serde_json::Value,
    ) {
        self.tool_calls.push(TurnToolCall {
            id: id.into(),
            name: name.into(),
            parameters: params,
            result: None,
            error: None,
        });
    }

    pub fn update_tool_call_parameters_for_id(
        &mut self,
        call_id: &str,
        parameters: serde_json::Value,
    ) -> bool {
        let Some(call) = self
            .tool_calls
            .iter_mut()
            .rev()
            .find(|call| call.id == call_id)
        else {
            return false;
        };
        call.parameters = parameters;
        true
    }

    /// Record tool call result.
    pub fn record_tool_result(&mut self, result: serde_json::Value) {
        if let Some(call) = self.tool_calls.last_mut() {
            call.result = Some(result);
            call.error = None;
        }
    }

    /// Attach a result to its exact tool call. Returns `false` when the call
    /// was not registered, allowing callers to surface protocol corruption.
    pub fn record_tool_result_for_id(&mut self, call_id: &str, result: serde_json::Value) -> bool {
        let Some(call) = self
            .tool_calls
            .iter_mut()
            .rev()
            .find(|call| call.id == call_id)
        else {
            return false;
        };
        call.result = Some(result);
        call.error = None;
        true
    }

    /// Record tool call error.
    pub fn record_tool_error(&mut self, error: impl Into<String>) {
        if let Some(call) = self.tool_calls.last_mut() {
            call.error = Some(error.into());
            call.result = None;
        }
    }

    /// Attach an error to its exact tool call.
    pub fn record_tool_error_for_id(&mut self, call_id: &str, error: impl Into<String>) -> bool {
        let Some(call) = self
            .tool_calls
            .iter_mut()
            .rev()
            .find(|call| call.id == call_id)
        else {
            return false;
        };
        call.error = Some(error.into());
        call.result = None;
        true
    }

    /// Append this turn's tool calls and their results to `messages` as a
    /// provider-valid assistant(tool_calls) + tool_result(...) sequence.
    ///
    /// No-op when the turn recorded no tool calls. See [`Thread::messages`] for
    /// the reconstruction contract; ids are synthesized as `t{turn}_c{index}`
    /// and are stable for a given turn/call position so repeated calls produce
    /// identical output. Provider-issued IDs are preserved when available;
    /// legacy calls without IDs receive stable synthetic IDs.
    pub(crate) fn append_tool_exchange(&self, messages: &mut Vec<ChatMessage>) {
        if self.tool_calls.is_empty() {
            return;
        }

        let mut calls = Vec::with_capacity(self.tool_calls.len());
        for (idx, call) in self.tool_calls.iter().enumerate() {
            calls.push(ToolCall {
                id: call.effective_id(self.turn_number, idx),
                name: call.name.clone(),
                arguments: call.parameters.clone(),
            });
        }
        messages.push(ChatMessage::assistant_with_tool_calls(None, calls));

        for (idx, call) in self.tool_calls.iter().enumerate() {
            let body = match (&call.result, &call.error) {
                (_, Some(error)) => {
                    format!("[error] {}", truncate_tool_body(error))
                }
                (Some(result), None) => {
                    let rendered = match result {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    truncate_tool_body(&rendered)
                }
                (None, None) => "[no result recorded]".to_string(),
            };
            messages.push(ChatMessage::tool_result(
                call.effective_id(self.turn_number, idx),
                &call.name,
                body,
            ));
        }
    }
}

/// Maximum characters retained for a single historical tool-result body when
/// reconstructing prior turns. Bounds context growth from one very large output
/// (e.g. a full file read or verbose test log) while keeping the head, which is
/// almost always the salient part for a follow-up turn.
const MAX_HISTORICAL_TOOL_RESULT_CHARS: usize = 4000;

/// Synthesize a stable tool-call id for a reconstructed prior turn.
fn synthetic_tool_call_id(turn_number: usize, call_index: usize) -> String {
    format!("t{turn_number}_c{call_index}")
}

/// Truncate a reconstructed tool-result body on a char boundary, appending a
/// marker noting how many characters were elided.
fn truncate_tool_body(body: &str) -> String {
    if body.chars().count() <= MAX_HISTORICAL_TOOL_RESULT_CHARS {
        return body.to_string();
    }
    let mut out: String = body
        .chars()
        .take(MAX_HISTORICAL_TOOL_RESULT_CHARS)
        .collect();
    let elided = body.chars().count() - MAX_HISTORICAL_TOOL_RESULT_CHARS;
    out.push_str(&format!(
        "\n… [truncated {elided} chars — full output in session history]"
    ));
    out
}

/// Record of a tool call made during a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnToolCall {
    /// Provider-issued call ID. Empty only for legacy serialized turns.
    #[serde(default)]
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Parameters passed to the tool.
    pub parameters: serde_json::Value,
    /// Result from the tool (if successful).
    pub result: Option<serde_json::Value>,
    /// Error from the tool (if failed).
    pub error: Option<String>,
}

pub const MAX_DURABLE_TOOL_CALLS: usize = 64;
const MAX_DURABLE_TOOL_RESULT_CHARS: usize = 16 * 1024;
const MAX_DURABLE_TOOL_ERROR_CHARS: usize = 4 * 1024;
const MAX_DURABLE_PARAMETER_KEYS: usize = 64;
const MAX_DURABLE_PARAMETER_KEY_CHARS: usize = 256;

/// Build a bounded, secret-minimized trace suitable for conversation metadata.
/// Exact arguments remain available during the live turn, but durable history
/// stores only their shape and digest so passwords, form input, tokens, and
/// other tool parameters are not copied into undo/runtime metadata.
pub fn durable_tool_trace(tool_calls: &[TurnToolCall]) -> Vec<TurnToolCall> {
    tool_calls
        .iter()
        .take(MAX_DURABLE_TOOL_CALLS)
        .map(|call| TurnToolCall {
            id: bounded_identifier(&call.id, 512),
            name: bounded_identifier(&call.name, 512),
            parameters: summarized_tool_parameters(&call.parameters),
            result: call
                .result
                .as_ref()
                .map(|result| bounded_durable_value(result, MAX_DURABLE_TOOL_RESULT_CHARS)),
            error: call
                .error
                .as_deref()
                .map(|error| bounded_durable_text(error, MAX_DURABLE_TOOL_ERROR_CHARS)),
        })
        .collect()
}

pub fn summarized_tool_parameters(parameters: &serde_json::Value) -> serde_json::Value {
    use sha2::{Digest, Sha256};

    // Rebuild known durable summaries instead of trusting the entire object.
    // Persisted metadata can be legacy, malformed, or externally modified; a
    // marker alone must not allow arbitrary fields (including secrets) to pass
    // through the redaction boundary.
    if let Some(summary) = normalized_existing_parameter_summary(parameters) {
        return summary;
    }

    let encoded = serde_json::to_vec(parameters).unwrap_or_default();
    let digest = hex::encode(Sha256::digest(&encoded));
    let (shape, keys, key_count) = match parameters {
        serde_json::Value::Object(values) => {
            let mut keys = values.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            let key_count = keys.len();
            keys.truncate(MAX_DURABLE_PARAMETER_KEYS);
            let keys = keys
                .into_iter()
                .map(|key| bounded_identifier(&key, MAX_DURABLE_PARAMETER_KEY_CHARS))
                .collect();
            ("object", keys, key_count)
        }
        serde_json::Value::Array(_) => ("array", Vec::new(), 0),
        serde_json::Value::String(_) => ("string", Vec::new(), 0),
        serde_json::Value::Number(_) => ("number", Vec::new(), 0),
        serde_json::Value::Bool(_) => ("boolean", Vec::new(), 0),
        serde_json::Value::Null => ("null", Vec::new(), 0),
    };
    serde_json::json!({
        "_thinclaw_parameter_values_redacted": true,
        "shape": shape,
        "keys": keys,
        "key_count": key_count,
        "sha256": digest,
        "encoded_bytes": encoded.len(),
    })
}

fn normalized_existing_parameter_summary(
    parameters: &serde_json::Value,
) -> Option<serde_json::Value> {
    let object = parameters.as_object()?;
    if object
        .get("_thinclaw_parameter_values_redacted")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return None;
    }
    let shape = object.get("shape")?.as_str()?;
    if !matches!(
        shape,
        "object" | "array" | "string" | "number" | "boolean" | "null"
    ) {
        return None;
    }
    let digest = object.get("sha256")?.as_str()?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let encoded_bytes = object.get("encoded_bytes")?.as_u64()?;
    let raw_keys = object
        .get("keys")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let key_count = object
        .get("key_count")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(raw_keys.len() as u64);
    let keys = raw_keys
        .into_iter()
        .filter_map(|key| key.as_str().map(ToOwned::to_owned))
        .take(MAX_DURABLE_PARAMETER_KEYS)
        .map(|key| bounded_identifier(&key, MAX_DURABLE_PARAMETER_KEY_CHARS))
        .collect::<Vec<_>>();
    Some(serde_json::json!({
        "_thinclaw_parameter_values_redacted": true,
        "shape": shape,
        "keys": keys,
        "key_count": key_count,
        "sha256": digest.to_ascii_lowercase(),
        "encoded_bytes": encoded_bytes,
    }))
}

fn bounded_identifier(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        value.chars().take(max_chars).collect()
    }
}

fn bounded_durable_value(value: &serde_json::Value, max_chars: usize) -> serde_json::Value {
    let rendered = match value {
        serde_json::Value::String(value) => value.clone(),
        value => value.to_string(),
    };
    if rendered.chars().count() <= max_chars {
        return value.clone();
    }
    use sha2::{Digest, Sha256};
    serde_json::json!({
        "_thinclaw_truncated": true,
        "preview": rendered.chars().take(max_chars).collect::<String>(),
        "original_chars": rendered.chars().count(),
        "sha256": hex::encode(Sha256::digest(rendered.as_bytes())),
    })
}

fn bounded_durable_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    use sha2::{Digest, Sha256};
    format!(
        "{}\n… [truncated; original_chars={}; sha256={}]",
        value.chars().take(max_chars).collect::<String>(),
        value.chars().count(),
        hex::encode(Sha256::digest(value.as_bytes()))
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnContextEvidence {
    pub segment_id: String,
    pub source: String,
    pub content: String,
}

pub const MAX_TURN_CONTEXT_EVIDENCE_ITEMS: usize = 64;
pub const MAX_TURN_CONTEXT_EVIDENCE_CHARS: usize = 64 * 1024;
const MAX_TURN_CONTEXT_SEGMENT_ID_CHARS: usize = 512;
const MAX_TURN_CONTEXT_SOURCE_CHARS: usize = 1_024;

/// Normalize untrusted evidence at every persistence/restart boundary. Live
/// attachment extraction already applies the same aggregate content budget,
/// but hydration and undo must not trust legacy metadata to have done so.
pub fn bounded_turn_context_evidence(contexts: &[TurnContextEvidence]) -> Vec<TurnContextEvidence> {
    let mut bounded = Vec::new();
    let mut remaining_chars = MAX_TURN_CONTEXT_EVIDENCE_CHARS;
    for context in contexts.iter().take(MAX_TURN_CONTEXT_EVIDENCE_ITEMS) {
        let Some(context) = bounded_context_evidence_item(context, remaining_chars) else {
            continue;
        };
        remaining_chars = remaining_chars.saturating_sub(context.content.chars().count());
        bounded.push(context);
        if remaining_chars == 0 {
            break;
        }
    }
    bounded
}

fn bounded_context_evidence_item(
    context: &TurnContextEvidence,
    remaining_chars: usize,
) -> Option<TurnContextEvidence> {
    if remaining_chars == 0 {
        return None;
    }
    let content = context
        .content
        .chars()
        .take(remaining_chars)
        .collect::<String>();
    if content.trim().is_empty() {
        return None;
    }
    Some(TurnContextEvidence {
        segment_id: bounded_identifier(&context.segment_id, MAX_TURN_CONTEXT_SEGMENT_ID_CHARS),
        source: bounded_identifier(&context.source, MAX_TURN_CONTEXT_SOURCE_CHARS),
        content,
    })
}

impl TurnToolCall {
    fn effective_id(&self, turn_number: usize, call_index: usize) -> String {
        if self.id.is_empty() {
            synthetic_tool_call_id(turn_number, call_index)
        } else {
            self.id.clone()
        }
    }
}

#[cfg(test)]
mod tests;
