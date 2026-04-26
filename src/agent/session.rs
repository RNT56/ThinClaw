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

use crate::agent::context_monitor::ContextPressure;
use crate::agent::personality::SessionPersonalityOverlay;
use crate::identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
use crate::llm::{ChatMessage, ToolCall};

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

    fn channel_tool_approval_key(channel: &str, tool_name: &str) -> String {
        format!("channel:{channel}:tool:{tool_name}")
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
                if self.threads.contains_key(&id) {
                    // Safe: contains_key confirmed the entry exists.
                    self.threads
                        .get_mut(&id)
                        .expect("contains_key confirmed entry exists")
                } else {
                    // Stale active_thread ID: create a new thread, which
                    // updates self.active_thread to the new thread's ID.
                    self.create_thread()
                }
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
}

fn default_true() -> bool {
    true
}

/// Persisted record of a sub-agent that was active for a conversation thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSubagentState {
    pub agent_id: Uuid,
    pub name: String,
    pub request: crate::agent::subagent_executor::SubagentSpawnRequest,
    pub channel_name: String,
    #[serde(default)]
    pub channel_metadata: serde_json::Value,
    pub parent_user_id: String,
    #[serde(default)]
    pub parent_identity: Option<ResolvedIdentity>,
    pub parent_thread_id: String,
    #[serde(default = "default_true")]
    pub reinject_result: bool,
}

/// Durable runtime envelope stored in conversation metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadRuntimeState {
    #[serde(default)]
    pub state: ThreadState,
    #[serde(default)]
    pub pending_approval: Option<PendingApproval>,
    #[serde(default)]
    pub pending_auth: Option<PendingAuth>,
    #[serde(default)]
    pub owner_agent_id: Option<String>,
    #[serde(default)]
    pub model_override: Option<crate::tools::builtin::llm_tools::ModelOverride>,
    #[serde(default)]
    pub auto_approved_tools: Vec<String>,
    #[serde(default)]
    pub active_subagents: Vec<PersistedSubagentState>,
    #[serde(default)]
    pub last_context_pressure: Option<ContextPressure>,
    #[serde(default)]
    pub post_compaction_context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_workspace_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen_provider_system_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_snapshot_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral_overlay_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_segment_order: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_context_refs: Vec<String>,
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
        }
    }

    pub fn runtime_state(
        &self,
        owner_agent_id: Option<String>,
        model_override: Option<crate::tools::builtin::llm_tools::ModelOverride>,
        auto_approved_tools: impl IntoIterator<Item = String>,
        active_subagents: Vec<PersistedSubagentState>,
        last_context_pressure: Option<ContextPressure>,
    ) -> ThreadRuntimeState {
        let mut auto_approved_tools: Vec<String> = auto_approved_tools.into_iter().collect();
        auto_approved_tools.sort();
        auto_approved_tools.dedup();
        ThreadRuntimeState {
            state: self.state,
            pending_approval: self.pending_approval.clone(),
            pending_auth: self.pending_auth.clone(),
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
            prompt_segment_order: Vec::new(),
            provider_context_refs: Vec::new(),
        }
    }

    pub fn restore_runtime_state(&mut self, runtime: ThreadRuntimeState) {
        self.pending_approval = runtime.pending_approval;
        self.pending_auth = runtime.pending_auth;
        self.state = match runtime.state {
            ThreadState::Processing => {
                if let Some(turn) = self.turns.last_mut() {
                    turn.interrupt();
                }
                ThreadState::Interrupted
            }
            other => other,
        };
        if self.pending_approval.is_some() {
            self.state = ThreadState::AwaitingApproval;
        }
        self.updated_at = Utc::now();
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
        if let Some(turn) = self.turns.last_mut() {
            turn.complete(response);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Fail the current turn with an error.
    pub fn fail_turn(&mut self, error: impl Into<String>) {
        if let Some(turn) = self.turns.last_mut() {
            turn.fail(error);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
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
    pub fn enter_auth_mode(&mut self, extension_name: String, auth_mode: PendingAuthMode) {
        self.pending_auth = Some(PendingAuth {
            extension_name,
            auth_mode,
        });
        self.updated_at = Utc::now();
    }

    /// Take the pending auth (clearing auth mode).
    pub fn take_pending_auth(&mut self) -> Option<PendingAuth> {
        self.pending_auth.take()
    }

    /// Interrupt the current turn.
    pub fn interrupt(&mut self) {
        if let Some(turn) = self.turns.last_mut() {
            turn.interrupt();
        }
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

    /// Get all messages for context building.
    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();
        for turn in &self.turns {
            messages.push(ChatMessage::user(&turn.user_input));
            if let Some(ref response) = turn.response {
                messages.push(ChatMessage::assistant(response));
            }
        }
        messages
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
    /// Clears existing turns and rebuilds from message pairs.
    /// Messages should alternate: user, assistant, user, assistant...
    pub fn restore_from_messages(&mut self, messages: Vec<ChatMessage>) {
        self.turns.clear();
        self.state = ThreadState::Idle;
        self.pending_approval = None;
        self.pending_auth = None;

        // Messages alternate: user, assistant, user, assistant...
        let mut iter = messages.into_iter().peekable();
        let mut turn_number = 0;

        while let Some(msg) = iter.next() {
            if msg.role == crate::llm::Role::User {
                let mut turn = Turn::new(turn_number, &msg.content, false);

                // Check if next is assistant response
                if let Some(next) = iter.peek()
                    && next.role == crate::llm::Role::Assistant
                {
                    // iter.next() is guaranteed Some after a successful peek()
                    if let Some(response) = iter.next() {
                        turn.complete(&response.content);
                    }
                }

                self.turns.push(turn);
                turn_number += 1;
            }
        }

        self.updated_at = Utc::now();
    }

    /// Restore thread state from DB conversation messages, preserving
    /// per-message visibility metadata used by the WebUI.
    pub fn restore_from_conversation_messages(
        &mut self,
        messages: &[crate::history::ConversationMessage],
    ) {
        self.turns.clear();
        self.state = ThreadState::Idle;
        self.pending_approval = None;
        self.pending_auth = None;

        let mut iter = messages.iter().peekable();
        let mut turn_number = 0;

        while let Some(msg) = iter.next() {
            if msg.role != "user" {
                if msg.role == "assistant" && message_is_startup_hook(&msg.metadata) {
                    let mut turn = Turn::new(turn_number, "", true);
                    turn.complete(&msg.content);
                    self.turns.push(turn);
                    turn_number += 1;
                }
                continue;
            }

            let hide_user_input_from_ui = message_hides_user_input_in_main_chat(&msg.metadata);
            let mut turn = Turn::new(turn_number, &msg.content, hide_user_input_from_ui);

            if let Some(next) = iter.peek()
                && next.role == "assistant"
                && let Some(response) = iter.next()
            {
                turn.complete(&response.content);
            }

            if turn.hide_user_input_from_ui && turn.response.is_none() {
                continue;
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
    /// Agent response (if completed).
    pub response: Option<String>,
    /// Tool calls made during this turn.
    pub tool_calls: Vec<TurnToolCall>,
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
            response: None,
            tool_calls: Vec::new(),
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
        self.tool_calls.push(TurnToolCall {
            name: name.into(),
            parameters: params,
            result: None,
            error: None,
        });
    }

    /// Record tool call result.
    pub fn record_tool_result(&mut self, result: serde_json::Value) {
        if let Some(call) = self.tool_calls.last_mut() {
            call.result = Some(result);
        }
    }

    /// Record tool call error.
    pub fn record_tool_error(&mut self, error: impl Into<String>) {
        if let Some(call) = self.tool_calls.last_mut() {
            call.error = Some(error.into());
        }
    }
}

fn message_hides_user_input_in_main_chat(metadata: &serde_json::Value) -> bool {
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

fn message_is_startup_hook(metadata: &serde_json::Value) -> bool {
    metadata
        .get("synthetic_origin")
        .and_then(|value| value.as_str())
        == Some("startup_hook")
}

/// Record of a tool call made during a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnToolCall {
    /// Tool name.
    pub name: String,
    /// Parameters passed to the tool.
    pub parameters: serde_json::Value,
    /// Result from the tool (if successful).
    pub result: Option<serde_json::Value>,
    /// Error from the tool (if failed).
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let mut session = Session::new("user-123");
        assert!(session.active_thread.is_none());

        session.create_thread();
        assert!(session.active_thread.is_some());
    }

    #[test]
    fn test_thread_turns() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Hello");
        assert_eq!(thread.state, ThreadState::Processing);
        assert_eq!(thread.turns.len(), 1);

        thread.complete_turn("Hi there!");
        assert_eq!(thread.state, ThreadState::Idle);
        assert_eq!(thread.turns[0].response, Some("Hi there!".to_string()));
    }

    #[test]
    fn test_thread_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("First message");
        thread.complete_turn("First response");
        thread.start_turn("Second message");
        thread.complete_turn("Second response");

        let messages = thread.messages();
        assert_eq!(messages.len(), 4);
    }

    #[test]
    fn test_turn_tool_calls() {
        let mut turn = Turn::new(0, "Test input", false);
        turn.record_tool_call("echo", serde_json::json!({"message": "test"}));
        turn.record_tool_result(serde_json::json!("test"));

        assert_eq!(turn.tool_calls.len(), 1);
        assert!(turn.tool_calls[0].result.is_some());
    }

    #[test]
    fn test_restore_from_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        // First add some turns
        thread.start_turn("Original message");
        thread.complete_turn("Original response");

        // Now restore from different messages
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
            ChatMessage::user("How are you?"),
            ChatMessage::assistant("I'm good!"),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].user_input, "Hello");
        assert_eq!(thread.turns[0].response, Some("Hi there!".to_string()));
        assert_eq!(thread.turns[1].user_input, "How are you?");
        assert_eq!(thread.turns[1].response, Some("I'm good!".to_string()));
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_restore_from_messages_incomplete_turn() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Messages with incomplete last turn (no assistant response)
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
            ChatMessage::user("How are you?"),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[1].user_input, "How are you?");
        assert!(thread.turns[1].response.is_none());
    }

    #[test]
    fn test_enter_auth_mode() {
        let mut thread = Thread::new(Uuid::new_v4());
        assert!(thread.pending_auth.is_none());

        thread.enter_auth_mode("telegram".to_string(), PendingAuthMode::ManualToken);
        assert!(thread.pending_auth.is_some());
        assert_eq!(
            thread.pending_auth.as_ref().unwrap().extension_name,
            "telegram"
        );
        assert_eq!(
            thread.pending_auth.as_ref().unwrap().auth_mode,
            PendingAuthMode::ManualToken
        );
    }

    #[test]
    fn test_take_pending_auth() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.enter_auth_mode("notion".to_string(), PendingAuthMode::ManualToken);

        let pending = thread.take_pending_auth();
        assert!(pending.is_some());
        assert_eq!(pending.unwrap().extension_name, "notion");

        // Should be cleared after take
        assert!(thread.pending_auth.is_none());
        assert!(thread.take_pending_auth().is_none());
    }

    #[test]
    fn test_pending_auth_serialization() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.enter_auth_mode("openai".to_string(), PendingAuthMode::ExternalOAuth);

        let json = serde_json::to_string(&thread).expect("should serialize");
        assert!(json.contains("pending_auth"));
        assert!(json.contains("openai"));

        let restored: Thread = serde_json::from_str(&json).expect("should deserialize");
        assert!(restored.pending_auth.is_some());
        let pending = restored.pending_auth.unwrap();
        assert_eq!(pending.extension_name, "openai");
        assert_eq!(pending.auth_mode, PendingAuthMode::ExternalOAuth);
    }

    #[test]
    fn test_pending_auth_default_none() {
        // Deserialization of old data without pending_auth should default to None
        let mut thread = Thread::new(Uuid::new_v4());
        thread.pending_auth = None;
        let json = serde_json::to_string(&thread).expect("serialize");

        // Remove the pending_auth field to simulate old data
        let json = json.replace(",\"pending_auth\":null", "");
        let restored: Thread = serde_json::from_str(&json).expect("should deserialize");
        assert!(restored.pending_auth.is_none());
    }

    #[test]
    fn test_runtime_state_roundtrip_preserves_resume_fields() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("inspect restart handling");
        thread.state = ThreadState::AwaitingApproval;
        thread.pending_approval = Some(PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"cmd": "pwd"}),
            description: "inspect workspace".to_string(),
            tool_call_id: "call_runtime".to_string(),
            context_messages: vec![ChatMessage::user("inspect restart handling")],
            deferred_tool_calls: vec![],
        });
        thread.pending_auth = Some(PendingAuth {
            extension_name: "github".to_string(),
            auth_mode: PendingAuthMode::ManualToken,
        });

        let runtime = thread.runtime_state(
            Some("agent-ops".to_string()),
            Some(crate::tools::builtin::llm_tools::ModelOverride {
                model_spec: "openai/gpt-4.1".to_string(),
                reason: Some("need stronger reasoning".to_string()),
            }),
            vec!["shell".to_string(), "read_file".to_string()],
            vec![PersistedSubagentState {
                agent_id: Uuid::new_v4(),
                name: "background-check".to_string(),
                request: crate::agent::subagent_executor::SubagentSpawnRequest {
                    name: "background-check".to_string(),
                    task: "verify restart state".to_string(),
                    system_prompt: None,
                    model: None,
                    task_packet: None,
                    memory_mode: None,
                    tool_mode: None,
                    skill_mode: None,
                    tool_profile: None,
                    allowed_tools: Some(vec!["read_file".to_string()]),
                    allowed_skills: Some(vec!["github".to_string()]),
                    principal_id: Some("principal-1".to_string()),
                    actor_id: Some("actor-1".to_string()),
                    agent_workspace_id: None,
                    timeout_secs: Some(30),
                    wait: false,
                },
                channel_name: "gateway".to_string(),
                channel_metadata: serde_json::json!({"thread_id": "thread-1"}),
                parent_user_id: "principal-1".to_string(),
                parent_identity: None,
                parent_thread_id: "thread-1".to_string(),
                reinject_result: true,
            }],
            Some(ContextPressure::Warning),
        );

        let json = serde_json::to_value(&runtime).expect("serialize runtime");
        let restored: ThreadRuntimeState =
            serde_json::from_value(json).expect("deserialize runtime");

        assert_eq!(restored.state, ThreadState::AwaitingApproval);
        assert_eq!(
            restored
                .pending_auth
                .as_ref()
                .map(|auth| auth.extension_name.as_str()),
            Some("github")
        );
        assert_eq!(restored.owner_agent_id.as_deref(), Some("agent-ops"));
        assert_eq!(
            restored
                .model_override
                .as_ref()
                .map(|m| m.model_spec.as_str()),
            Some("openai/gpt-4.1")
        );
        assert_eq!(
            restored.auto_approved_tools,
            vec!["read_file".to_string(), "shell".to_string()]
        );
        assert_eq!(restored.active_subagents.len(), 1);
        assert_eq!(
            restored.last_context_pressure,
            Some(ContextPressure::Warning)
        );
        assert_eq!(
            restored.active_subagents[0]
                .request
                .allowed_skills
                .as_deref(),
            Some(&["github".to_string()][..])
        );
    }

    #[test]
    fn test_restore_runtime_state_interrupts_processing_turns_on_resume() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("long-running work");

        thread.restore_runtime_state(ThreadRuntimeState {
            state: ThreadState::Processing,
            pending_approval: None,
            pending_auth: None,
            owner_agent_id: None,
            model_override: None,
            auto_approved_tools: vec![],
            active_subagents: vec![],
            last_context_pressure: None,
            post_compaction_context: None,
            frozen_workspace_prompt: None,
            frozen_provider_system_prompt: None,
            prompt_snapshot_hash: None,
            ephemeral_overlay_hash: None,
            prompt_segment_order: Vec::new(),
            provider_context_refs: Vec::new(),
        });

        assert_eq!(thread.state, ThreadState::Interrupted);
        assert_eq!(
            thread.last_turn().map(|turn| turn.state),
            Some(TurnState::Interrupted)
        );
    }

    #[test]
    fn test_thread_runtime_state_serde_round_trip_preserves_prompt_fields() {
        let runtime = ThreadRuntimeState {
            state: ThreadState::Idle,
            pending_approval: None,
            pending_auth: None,
            owner_agent_id: Some("agent-1".to_string()),
            model_override: None,
            auto_approved_tools: vec!["shell".to_string()],
            active_subagents: Vec::new(),
            last_context_pressure: Some(ContextPressure::Warning),
            post_compaction_context: Some("summary".to_string()),
            frozen_workspace_prompt: Some("workspace".to_string()),
            frozen_provider_system_prompt: Some("provider".to_string()),
            prompt_snapshot_hash: Some("sha256:stable".to_string()),
            ephemeral_overlay_hash: Some("sha256:ephemeral".to_string()),
            prompt_segment_order: vec![
                "stable:identity".to_string(),
                "ephemeral:provider_recall".to_string(),
            ],
            provider_context_refs: vec!["provider:1".to_string(), "provider:2".to_string()],
        };

        let encoded = serde_json::to_string(&runtime).expect("serialize runtime");
        let decoded: ThreadRuntimeState =
            serde_json::from_str(&encoded).expect("deserialize runtime");

        assert_eq!(decoded.prompt_snapshot_hash, runtime.prompt_snapshot_hash);
        assert_eq!(
            decoded.frozen_workspace_prompt,
            runtime.frozen_workspace_prompt
        );
        assert_eq!(
            decoded.frozen_provider_system_prompt,
            runtime.frozen_provider_system_prompt
        );
        assert_eq!(
            decoded.ephemeral_overlay_hash,
            runtime.ephemeral_overlay_hash
        );
        assert_eq!(decoded.prompt_segment_order, runtime.prompt_segment_order);
        assert_eq!(decoded.provider_context_refs, runtime.provider_context_refs);
    }

    #[test]
    fn test_thread_with_id() {
        let specific_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let thread = Thread::with_id(specific_id, session_id);

        assert_eq!(thread.id, specific_id);
        assert_eq!(thread.session_id, session_id);
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_thread_with_id_restore_messages() {
        let thread_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut thread = Thread::with_id(thread_id, session_id);

        let messages = vec![
            ChatMessage::user("Hello from DB"),
            ChatMessage::assistant("Restored response"),
        ];
        thread.restore_from_messages(messages);

        assert_eq!(thread.id, thread_id);
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].user_input, "Hello from DB");
        assert_eq!(
            thread.turns[0].response,
            Some("Restored response".to_string())
        );
    }

    #[test]
    fn test_restore_from_messages_empty() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Add a turn first, then restore with empty vec
        thread.start_turn("hello");
        thread.complete_turn("hi");
        assert_eq!(thread.turns.len(), 1);

        thread.restore_from_messages(Vec::new());

        // Should clear all turns and stay idle
        assert!(thread.turns.is_empty());
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_restore_from_messages_only_assistant_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Only assistant messages (no user messages to anchor turns)
        let messages = vec![
            ChatMessage::assistant("I'm here"),
            ChatMessage::assistant("Still here"),
        ];

        thread.restore_from_messages(messages);

        // Assistant-only messages have no user turn to attach to, so
        // they should be skipped entirely.
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_restore_from_conversation_messages_hides_only_startup_user_prompt() {
        let mut thread = Thread::new(Uuid::new_v4());
        let now = Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "boot prompt".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({"hide_from_webui_chat": true}),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "boot reply".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({"synthetic_origin": "startup_hook"}),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "real question".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "real answer".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(3),
            },
        ];

        thread.restore_from_conversation_messages(&messages);

        assert_eq!(thread.turns.len(), 2);
        assert!(thread.turns[0].hide_user_input_from_ui);
        assert_eq!(thread.turns[0].response.as_deref(), Some("boot reply"));
        assert_eq!(thread.turns[1].user_input, "real question");
        assert!(!thread.turns[1].hide_user_input_from_ui);
    }

    #[test]
    fn test_restore_from_conversation_messages_preserves_legacy_assistant_only_startup_reply() {
        let mut thread = Thread::new(Uuid::new_v4());
        let now = Utc::now();
        let messages = vec![crate::history::ConversationMessage {
            id: Uuid::new_v4(),
            role: "assistant".to_string(),
            content: "boot reply".to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({"synthetic_origin": "startup_hook"}),
            created_at: now,
        }];

        thread.restore_from_conversation_messages(&messages);

        assert_eq!(thread.turns.len(), 1);
        assert!(thread.turns[0].hide_user_input_from_ui);
        assert_eq!(thread.turns[0].user_input, "");
        assert_eq!(thread.turns[0].response.as_deref(), Some("boot reply"));
    }

    #[test]
    fn test_restore_from_messages_multiple_user_messages_in_a_row() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Two user messages with no assistant response between them
        let messages = vec![
            ChatMessage::user("first"),
            ChatMessage::user("second"),
            ChatMessage::assistant("reply to second"),
        ];

        thread.restore_from_messages(messages);

        // First user message becomes a turn with no response,
        // second user message pairs with the assistant response.
        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].user_input, "first");
        assert!(thread.turns[0].response.is_none());
        assert_eq!(thread.turns[1].user_input, "second");
        assert_eq!(
            thread.turns[1].response,
            Some("reply to second".to_string())
        );
    }

    #[test]
    fn test_thread_switch() {
        let mut session = Session::new("user-1");

        let t1_id = session.create_thread().id;
        let t2_id = session.create_thread().id;

        // After creating two threads, active should be the last one
        assert_eq!(session.active_thread, Some(t2_id));

        // Switch back to the first
        assert!(session.switch_thread(t1_id));
        assert_eq!(session.active_thread, Some(t1_id));

        // Switching to a nonexistent thread should fail
        let fake_id = Uuid::new_v4();
        assert!(!session.switch_thread(fake_id));
        // Active thread should remain unchanged
        assert_eq!(session.active_thread, Some(t1_id));
    }

    #[test]
    fn test_get_or_create_thread_idempotent() {
        let mut session = Session::new("user-1");

        let tid1 = session.get_or_create_thread().id;
        let tid2 = session.get_or_create_thread().id;

        // Should return the same thread (not create a new one each time)
        assert_eq!(tid1, tid2);
        assert_eq!(session.threads.len(), 1);
    }

    #[test]
    fn test_truncate_turns() {
        let mut thread = Thread::new(Uuid::new_v4());

        for i in 0..5 {
            thread.start_turn(format!("msg-{}", i));
            thread.complete_turn(format!("resp-{}", i));
        }
        assert_eq!(thread.turns.len(), 5);

        thread.truncate_turns(3);
        assert_eq!(thread.turns.len(), 3);

        // Should keep the most recent turns
        assert_eq!(thread.turns[0].user_input, "msg-2");
        assert_eq!(thread.turns[1].user_input, "msg-3");
        assert_eq!(thread.turns[2].user_input, "msg-4");

        // Turn numbers should be re-indexed
        assert_eq!(thread.turns[0].turn_number, 0);
        assert_eq!(thread.turns[1].turn_number, 1);
        assert_eq!(thread.turns[2].turn_number, 2);
    }

    #[test]
    fn test_truncate_turns_noop_when_fewer() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("only one");
        thread.complete_turn("response");

        thread.truncate_turns(10);
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].user_input, "only one");
    }

    #[test]
    fn test_thread_interrupt_and_resume() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("do something");
        assert_eq!(thread.state, ThreadState::Processing);

        thread.interrupt();
        assert_eq!(thread.state, ThreadState::Interrupted);

        let last_turn = thread.last_turn().unwrap();
        assert_eq!(last_turn.state, TurnState::Interrupted);
        assert!(last_turn.completed_at.is_some());

        thread.resume();
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_resume_only_from_interrupted() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Idle thread: resume should be a no-op
        assert_eq!(thread.state, ThreadState::Idle);
        thread.resume();
        assert_eq!(thread.state, ThreadState::Idle);

        // Processing thread: resume should not change state
        thread.start_turn("work");
        assert_eq!(thread.state, ThreadState::Processing);
        thread.resume();
        assert_eq!(thread.state, ThreadState::Processing);
    }

    #[test]
    fn test_turn_fail() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("risky operation");
        thread.fail_turn("connection timed out");

        assert_eq!(thread.state, ThreadState::Idle);

        let turn = thread.last_turn().unwrap();
        assert_eq!(turn.state, TurnState::Failed);
        assert_eq!(turn.error, Some("connection timed out".to_string()));
        assert!(turn.response.is_none());
        assert!(turn.completed_at.is_some());
    }

    #[test]
    fn test_messages_with_incomplete_last_turn() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("first");
        thread.complete_turn("first reply");
        thread.start_turn("second (in progress)");

        let messages = thread.messages();
        // Should have 3 messages: user, assistant, user (no assistant for in-progress)
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[1].content, "first reply");
        assert_eq!(messages[2].content, "second (in progress)");
    }

    #[test]
    fn test_thread_serialization_round_trip() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("hello");
        thread.complete_turn("world");

        let json = serde_json::to_string(&thread).unwrap();
        let restored: Thread = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.id, thread.id);
        assert_eq!(restored.session_id, thread.session_id);
        assert_eq!(restored.turns.len(), 1);
        assert_eq!(restored.turns[0].user_input, "hello");
        assert_eq!(restored.turns[0].response, Some("world".to_string()));
    }

    #[test]
    fn test_session_serialization_round_trip() {
        let mut session = Session::new("user-ser");
        session.create_thread();
        session.auto_approve_tool("echo");

        let json = serde_json::to_string(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.user_id, "user-ser");
        assert_eq!(restored.threads.len(), 1);
        assert!(restored.is_tool_auto_approved("echo"));
        assert!(!restored.is_tool_auto_approved("shell"));
    }

    #[test]
    fn test_auto_approved_tools() {
        let mut session = Session::new("user-1");

        assert!(!session.is_tool_auto_approved("shell"));
        session.auto_approve_tool("shell");
        assert!(session.is_tool_auto_approved("shell"));

        // Idempotent
        session.auto_approve_tool("shell");
        assert_eq!(session.auto_approved_tools.len(), 1);
    }

    #[test]
    fn test_channel_scoped_auto_approval() {
        let mut session = Session::new("user-chan");

        session.auto_approve_tool_for_channel("gateway", "shell");
        assert!(session.is_tool_auto_approved_for_channel("gateway", "shell"));
        assert!(!session.is_tool_auto_approved_for_channel("telegram", "shell"));
    }

    #[test]
    fn test_legacy_global_auto_approval_still_applies() {
        let mut session = Session::new("user-legacy");

        session.auto_approve_tool("http");
        assert!(session.is_tool_auto_approved_for_channel("gateway", "http"));
        assert!(session.is_tool_auto_approved_for_channel("telegram", "http"));
    }

    #[test]
    fn test_turn_tool_call_error() {
        let mut turn = Turn::new(0, "test", false);
        turn.record_tool_call("http", serde_json::json!({"url": "example.com"}));
        turn.record_tool_error("timeout");

        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].error, Some("timeout".to_string()));
        assert!(turn.tool_calls[0].result.is_none());
    }

    #[test]
    fn test_turn_number_increments() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Before any turns, turn_number() is 1 (1-indexed for display)
        assert_eq!(thread.turn_number(), 1);

        thread.start_turn("first");
        thread.complete_turn("done");
        assert_eq!(thread.turn_number(), 2);

        thread.start_turn("second");
        assert_eq!(thread.turn_number(), 3);
    }

    #[test]
    fn test_complete_turn_on_empty_thread() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Completing a turn when there are no turns should be a safe no-op
        thread.complete_turn("phantom response");
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_fail_turn_on_empty_thread() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Failing a turn when there are no turns should be a safe no-op
        thread.fail_turn("phantom error");
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_pending_approval_flow() {
        let mut thread = Thread::new(Uuid::new_v4());

        let approval = PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"command": "rm -rf /"}),
            description: "dangerous command".to_string(),
            tool_call_id: "call_123".to_string(),
            context_messages: vec![ChatMessage::user("do it")],
            deferred_tool_calls: vec![],
        };

        thread.await_approval(approval);
        assert_eq!(thread.state, ThreadState::AwaitingApproval);
        assert!(thread.pending_approval.is_some());

        let taken = thread.take_pending_approval();
        assert!(taken.is_some());
        assert_eq!(taken.unwrap().tool_name, "shell");
        assert!(thread.pending_approval.is_none());
    }

    #[test]
    fn test_clear_pending_approval() {
        let mut thread = Thread::new(Uuid::new_v4());

        let approval = PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "http".to_string(),
            parameters: serde_json::json!({}),
            description: "test".to_string(),
            tool_call_id: "call_456".to_string(),
            context_messages: vec![],
            deferred_tool_calls: vec![],
        };

        thread.await_approval(approval);
        thread.clear_pending_approval();

        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.pending_approval.is_none());
    }

    #[test]
    fn test_active_thread_accessors() {
        let mut session = Session::new("user-1");

        assert!(session.active_thread().is_none());
        assert!(session.active_thread_mut().is_none());

        let tid = session.create_thread().id;

        assert!(session.active_thread().is_some());
        assert_eq!(session.active_thread().unwrap().id, tid);

        // Mutably modify through accessor
        session.active_thread_mut().unwrap().start_turn("test");
        assert_eq!(
            session.active_thread().unwrap().state,
            ThreadState::Processing
        );
    }
}
