//! Root-independent thread operation helpers.

use std::collections::HashSet;

use uuid::Uuid;

use thinclaw_llm_core::ChatMessage;

use crate::ports::{PortableThreadState, ThreadMessage, ThreadRuntimeSnapshot, ThreadStorePort};
use crate::session::{
    PendingApproval, PendingAuth, PendingAuthMode, Thread, ThreadState,
    message_hides_user_input_in_main_chat,
};
use crate::undo::{Checkpoint, UndoManager};
use thinclaw_types::error::DatabaseError;

pub const DIRECT_THREAD_ROLE_KEY: &str = "direct_thread_role";
pub const DIRECT_THREAD_ROLE_MAIN: &str = "main";
pub const ORIGIN_CHANNEL_KEY: &str = "origin_channel";
pub const LAST_ACTIVE_CHANNEL_KEY: &str = "last_active_channel";
pub const SEEN_CHANNELS_KEY: &str = "seen_channels";

pub fn detect_user_correction_signal(role: &str, content: &str) -> u32 {
    if !role.eq_ignore_ascii_case("user") {
        return 0;
    }
    let normalized = content.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return 0;
    }

    let correction_prefixes = [
        "actually",
        "correction:",
        "to clarify",
        "that's incorrect",
        "that is incorrect",
        "not quite",
        "no,",
        "no.",
        "use this instead",
        "please use",
        "instead:",
    ];
    if correction_prefixes
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
    {
        return 1;
    }

    let correction_markers = [
        "you should have",
        "please do not",
        "this is wrong",
        "the correct way is",
    ];
    if correction_markers
        .iter()
        .any(|marker| normalized.contains(marker))
    {
        return 1;
    }

    0
}

pub fn direct_thread_role_from_metadata(metadata: &serde_json::Value) -> Option<&str> {
    metadata
        .get(DIRECT_THREAD_ROLE_KEY)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub fn is_primary_direct_thread_metadata(metadata: &serde_json::Value) -> bool {
    direct_thread_role_from_metadata(metadata) == Some(DIRECT_THREAD_ROLE_MAIN)
        || metadata.get("thread_type").and_then(|value| value.as_str()) == Some("assistant")
}

pub fn direct_conversation_metadata_updates(
    metadata: &serde_json::Value,
    channel: &str,
    has_thread_id: bool,
) -> Vec<(&'static str, serde_json::Value)> {
    let mut updates = Vec::new();

    if direct_thread_role_from_metadata(metadata).is_none() && !has_thread_id {
        updates.push((
            DIRECT_THREAD_ROLE_KEY,
            serde_json::json!(DIRECT_THREAD_ROLE_MAIN),
        ));
    }

    if metadata
        .get(ORIGIN_CHANNEL_KEY)
        .is_none_or(|value| value.is_null())
    {
        updates.push((ORIGIN_CHANNEL_KEY, serde_json::json!(channel)));
    }

    updates.push((LAST_ACTIVE_CHANNEL_KEY, serde_json::json!(channel)));

    let mut seen_channels: Vec<String> = metadata
        .get(SEEN_CHANNELS_KEY)
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if !seen_channels.iter().any(|seen| seen == channel) {
        seen_channels.push(channel.to_string());
        seen_channels.sort();
        seen_channels.dedup();
        updates.push((SEEN_CHANNELS_KEY, serde_json::json!(seen_channels)));
    }

    updates
}

pub fn direct_conversation_candidate_is_primary(
    metadata: &serde_json::Value,
    thread_type: Option<&str>,
) -> bool {
    direct_thread_role_from_metadata(metadata) == Some(DIRECT_THREAD_ROLE_MAIN)
        || thread_type == Some("assistant")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadVisibilityDecision {
    Visible,
    CheckPrincipalUser,
    Hidden,
}

pub fn thread_visibility_after_actor_membership(
    principal_id: &str,
    actor_id: &str,
    belongs_to_actor: bool,
) -> ThreadVisibilityDecision {
    if belongs_to_actor {
        ThreadVisibilityDecision::Visible
    } else if actor_id == principal_id {
        ThreadVisibilityDecision::CheckPrincipalUser
    } else {
        ThreadVisibilityDecision::Hidden
    }
}

#[derive(Debug, Clone)]
pub struct PostCompactionFactAccumulator {
    facts: Vec<String>,
    seen: HashSet<String>,
    max_total: usize,
}

impl PostCompactionFactAccumulator {
    pub fn new(max_total: usize) -> Self {
        Self {
            facts: Vec::new(),
            seen: HashSet::new(),
            max_total,
        }
    }

    pub fn remaining(&self) -> usize {
        self.max_total.saturating_sub(self.facts.len())
    }

    pub fn extend_source<I>(&mut self, source: &str, candidates: I)
    where
        I: IntoIterator<Item = String>,
    {
        for candidate in candidates {
            if self.facts.len() >= self.max_total {
                break;
            }
            let decorated = format!("{source}: {candidate}");
            let key = decorated.trim().to_ascii_lowercase();
            if !key.is_empty() && self.seen.insert(key) {
                self.facts.push(decorated);
            }
        }
    }

    pub fn into_facts(self) -> Vec<String> {
        self.facts
    }
}

pub fn compact_text_preview(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview: String = collapsed.chars().take(120).collect();
    if collapsed.chars().count() > 120 {
        format!("{preview}…")
    } else {
        preview
    }
}

pub fn trajectory_learning_metadata(
    thread_id: Uuid,
    session_id: Option<Uuid>,
    turn_number: Option<usize>,
) -> serde_json::Value {
    let mut metadata = serde_json::json!({});
    if let Some(obj) = metadata.as_object_mut() {
        if let Some(session_id) = session_id {
            obj.insert(
                "session_id".to_string(),
                serde_json::json!(session_id.to_string()),
            );
        }
        if let Some(turn_number) = turn_number {
            obj.insert("turn_number".to_string(), serde_json::json!(turn_number));
        }
        if let (Some(session_id), Some(turn_number)) = (session_id, turn_number) {
            obj.insert(
                "trajectory_target_id".to_string(),
                serde_json::json!(format!("{session_id}:{thread_id}:{turn_number}")),
            );
        }
    }
    metadata
}

/// Build the durable runtime snapshot for a thread while preserving runtime
/// fields that are not owned by the live in-memory thread model.
pub fn runtime_snapshot_for_persistence(
    thread: &Thread,
    owner_agent_id: Option<String>,
    model_override: Option<crate::ports::ModelOverride>,
    auto_approved_tools: Option<Vec<String>>,
    active_subagents: Vec<crate::ports::PortableSubagentState>,
    existing_runtime: Option<&ThreadRuntimeSnapshot>,
) -> ThreadRuntimeSnapshot {
    let mut runtime = thread.runtime_snapshot(
        owner_agent_id,
        model_override,
        auto_approved_tools.unwrap_or_else(|| {
            existing_runtime
                .map(|runtime| runtime.auto_approved_tools.clone())
                .unwrap_or_default()
        }),
        active_subagents,
        existing_runtime.and_then(|runtime| runtime.last_context_pressure.clone()),
    );

    if let Some(existing) = existing_runtime {
        runtime.post_compaction_context = existing.post_compaction_context.clone();
        runtime.frozen_workspace_prompt = existing.frozen_workspace_prompt.clone();
        runtime.frozen_provider_system_prompt = existing.frozen_provider_system_prompt.clone();
        runtime.prompt_snapshot_hash = existing.prompt_snapshot_hash.clone();
        runtime.ephemeral_overlay_hash = existing.ephemeral_overlay_hash.clone();
        runtime.prompt_segment_order = existing.prompt_segment_order.clone();
        runtime.provider_context_refs = existing.provider_context_refs.clone();
        runtime.undo_checkpoints = existing.undo_checkpoints.clone();
    }
    // The active-message watermark always tracks the live in-memory thread:
    // every persisted turn grows it (1 row per user message, 1 more once the
    // assistant responds), and `/undo`/`/redo`/`/clear` set it explicitly via
    // `set_active_message_row_count` right after they mutate `thread.turns`.
    // Recomputing it here keeps normal turns from leaving a stale, too-small
    // watermark in place after an earlier undo.
    runtime.active_message_row_count = Some(thread.messages().len() as i64);

    runtime
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadInputAdmission {
    Accept,
    Reject(&'static str),
}

pub fn thread_input_admission(thread: &Thread) -> ThreadInputAdmission {
    thread_state_input_admission(thread.state)
}

pub fn thread_state_input_admission(state: ThreadState) -> ThreadInputAdmission {
    match state {
        ThreadState::Processing => {
            ThreadInputAdmission::Reject("Turn in progress. Use /interrupt to cancel.")
        }
        ThreadState::AwaitingApproval => {
            ThreadInputAdmission::Reject("Waiting for approval. Use /interrupt to cancel.")
        }
        ThreadState::Completed => {
            ThreadInputAdmission::Reject("Thread completed. Use /thread new.")
        }
        ThreadState::Idle | ThreadState::Interrupted => ThreadInputAdmission::Accept,
    }
}

#[derive(Debug, Clone)]
pub enum PendingApprovalAdmission {
    Ready(PendingApproval),
    Missing,
    RequestIdMismatch,
}

pub fn take_pending_approval_matching(
    thread: &mut Thread,
    request_id: Option<Uuid>,
) -> PendingApprovalAdmission {
    if thread.state != ThreadState::AwaitingApproval {
        return PendingApprovalAdmission::Missing;
    }

    let Some(pending) = thread.take_pending_approval() else {
        return PendingApprovalAdmission::Missing;
    };

    if let Some(request_id) = request_id
        && request_id != pending.request_id
    {
        thread.await_approval(pending);
        return PendingApprovalAdmission::RequestIdMismatch;
    }

    PendingApprovalAdmission::Ready(pending)
}

pub fn pending_approval_missing_message() -> &'static str {
    "No pending approval request."
}

pub fn pending_approval_request_mismatch_message() -> &'static str {
    "Request ID mismatch. Use the correct request ID."
}

pub fn mark_pending_approval_approved(thread: &mut Thread) {
    thread.state = ThreadState::Processing;
}

pub fn checkpoint_before_turn(thread: &Thread, undo: &mut UndoManager) {
    let turn_number = thread.turn_number();
    undo.checkpoint(
        turn_number,
        thread.messages(),
        format!("Before turn {turn_number}"),
    );
}

pub fn start_user_turn(
    thread: &mut Thread,
    undo: &mut UndoManager,
    content: &str,
    metadata: &serde_json::Value,
) -> Vec<ChatMessage> {
    checkpoint_before_turn(thread, undo);
    let hide_user_input_from_ui = message_hides_user_input_in_main_chat(metadata);
    thread.start_turn_with_visibility(content, hide_user_input_from_ui);
    thread.messages()
}

pub fn interrupt_thread(thread: &mut Thread) -> bool {
    match thread.state {
        ThreadState::Processing | ThreadState::AwaitingApproval => {
            thread.interrupt();
            true
        }
        _ => false,
    }
}

pub fn clear_thread(thread: &mut Thread) {
    thread.turns.clear();
    thread.state = ThreadState::Idle;
    thread.pending_approval = None;
    thread.pending_auth = None;
}

pub fn complete_thread_response(thread: &mut Thread, response: &str) -> (usize, Vec<ChatMessage>) {
    thread.complete_turn(response);
    (thread.turn_number(), thread.messages())
}

pub fn fail_thread_turn(thread: &mut Thread, error: &str) -> Vec<ChatMessage> {
    thread.fail_turn(error);
    thread.messages()
}

pub fn await_thread_approval(thread: &mut Thread, pending: PendingApproval) -> Vec<ChatMessage> {
    thread.await_approval(pending);
    thread.messages()
}

pub fn reject_pending_approval(thread: &mut Thread, rejection: &str) -> (usize, Vec<ChatMessage>) {
    thread.clear_pending_approval();
    thread.complete_turn(rejection);
    (thread.turn_number(), thread.messages())
}

pub fn enter_auth_mode_and_complete_turn(
    thread: &mut Thread,
    extension_name: String,
    auth_mode: PendingAuthMode,
    instructions: &str,
) -> (usize, Vec<ChatMessage>) {
    thread.enter_auth_mode(extension_name, auth_mode);
    thread.complete_turn(instructions);
    (thread.turn_number(), thread.messages())
}

pub fn clear_pending_auth(thread: &mut Thread) -> bool {
    thread.take_pending_auth().is_some()
}

pub fn reenter_pending_auth(thread: &mut Thread, pending: &PendingAuth) {
    thread.enter_auth_mode(pending.extension_name.clone(), pending.auth_mode);
}

pub fn auth_mode_status_label(mode: PendingAuthMode) -> &'static str {
    match mode {
        PendingAuthMode::ManualToken => "manual_token",
        PendingAuthMode::ExternalOAuth => "oauth",
    }
}

pub fn auth_required_status_mode(
    parsed_auth_mode: Option<String>,
    fallback_mode: PendingAuthMode,
) -> String {
    parsed_auth_mode.unwrap_or_else(|| auth_mode_status_label(fallback_mode).to_string())
}

pub fn auth_required_status(parsed_auth_status: Option<String>) -> String {
    parsed_auth_status.unwrap_or_else(|| "awaiting_token".to_string())
}

pub fn auth_activation_success_message(extension_name: &str, tools_loaded: &[String]) -> String {
    let tool_list = if tools_loaded.is_empty() {
        String::new()
    } else {
        format!("\n\nTools: {}", tools_loaded.join(", "))
    };
    format!(
        "{} authenticated and activated ({} tools loaded).{}",
        extension_name,
        tools_loaded.len(),
        tool_list
    )
}

pub fn auth_activation_failed_message(extension_name: &str, error: &str) -> String {
    format!(
        "{} authenticated successfully, but activation failed: {}. Try activating manually.",
        extension_name, error
    )
}

pub fn invalid_auth_token_message(instructions: Option<String>) -> String {
    instructions.unwrap_or_else(|| "Invalid token. Please try again.".to_string())
}

pub fn auth_failed_message(extension_name: &str, error: &str) -> String {
    format!("Authentication failed for {}: {}", extension_name, error)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoRedoOutcome {
    Restored {
        turn_number: usize,
        remaining: usize,
    },
    NothingAvailable,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoRedoAction {
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadOperationMessage {
    Ok(String),
    Error(&'static str),
}

pub fn undo_redo_message(
    action: UndoRedoAction,
    outcome: &UndoRedoOutcome,
) -> ThreadOperationMessage {
    match (action, outcome) {
        (
            UndoRedoAction::Undo,
            UndoRedoOutcome::Restored {
                turn_number,
                remaining,
            },
        ) => ThreadOperationMessage::Ok(format!(
            "Undone to turn {}. {} undo(s) remaining.",
            turn_number, remaining
        )),
        (UndoRedoAction::Redo, UndoRedoOutcome::Restored { turn_number, .. }) => {
            ThreadOperationMessage::Ok(format!("Redone to turn {}.", turn_number))
        }
        (UndoRedoAction::Undo, UndoRedoOutcome::NothingAvailable) => {
            ThreadOperationMessage::Ok("Nothing to undo.".to_string())
        }
        (UndoRedoAction::Redo, UndoRedoOutcome::NothingAvailable) => {
            ThreadOperationMessage::Ok("Nothing to redo.".to_string())
        }
        (UndoRedoAction::Undo, UndoRedoOutcome::Failed) => {
            ThreadOperationMessage::Error("Undo failed.")
        }
        (UndoRedoAction::Redo, UndoRedoOutcome::Failed) => {
            ThreadOperationMessage::Error("Redo failed.")
        }
    }
}

pub fn restore_thread_from_undo(thread: &mut Thread, undo: &mut UndoManager) -> UndoRedoOutcome {
    if !undo.can_undo() {
        return UndoRedoOutcome::NothingAvailable;
    }

    let current_messages = thread.messages();
    let current_turn = thread.turn_number();

    if let Some(checkpoint) = undo.undo(current_turn, current_messages) {
        let turn_number = checkpoint.turn_number;
        thread.restore_from_messages(checkpoint.messages);
        UndoRedoOutcome::Restored {
            turn_number,
            remaining: undo.undo_count(),
        }
    } else {
        UndoRedoOutcome::Failed
    }
}

pub fn restore_thread_from_redo(thread: &mut Thread, undo: &mut UndoManager) -> UndoRedoOutcome {
    if !undo.can_redo() {
        return UndoRedoOutcome::NothingAvailable;
    }

    let current_messages = thread.messages();
    let current_turn = thread.turn_number();

    if let Some(checkpoint) = undo.redo(current_turn, current_messages) {
        let turn_number = checkpoint.turn_number;
        thread.restore_from_messages(checkpoint.messages);
        UndoRedoOutcome::Restored {
            turn_number,
            remaining: undo.redo_count(),
        }
    } else {
        UndoRedoOutcome::Failed
    }
}

pub fn restore_thread_from_checkpoint(
    thread: &mut Thread,
    undo: &mut UndoManager,
    checkpoint_id: Uuid,
) -> Option<String> {
    let checkpoint = undo.restore(checkpoint_id)?;
    let description = checkpoint.description.clone();
    thread.restore_from_messages(checkpoint.messages);
    Some(description)
}

pub async fn mutate_thread_runtime_snapshot<F>(
    store: &dyn ThreadStorePort,
    thread_id: Uuid,
    mutate: F,
) -> Result<ThreadRuntimeSnapshot, DatabaseError>
where
    F: FnOnce(&mut ThreadRuntimeSnapshot),
{
    let mut runtime = store
        .load_thread_runtime(thread_id)
        .await?
        .unwrap_or_default();
    mutate(&mut runtime);
    store.save_thread_runtime(thread_id, &runtime).await?;
    Ok(runtime)
}

pub async fn set_post_compaction_context(
    store: &dyn ThreadStorePort,
    thread_id: Uuid,
    fragment: Option<String>,
) -> Result<ThreadRuntimeSnapshot, DatabaseError> {
    mutate_thread_runtime_snapshot(store, thread_id, |runtime| {
        runtime.post_compaction_context = fragment.clone();
    })
    .await
}

pub async fn load_last_context_pressure(
    store: &dyn ThreadStorePort,
    thread_id: Uuid,
) -> Result<Option<serde_json::Value>, DatabaseError> {
    Ok(store
        .load_thread_runtime(thread_id)
        .await?
        .and_then(|runtime| runtime.last_context_pressure))
}

pub async fn set_last_context_pressure(
    store: &dyn ThreadStorePort,
    thread_id: Uuid,
    pressure: Option<serde_json::Value>,
) -> Result<ThreadRuntimeSnapshot, DatabaseError> {
    mutate_thread_runtime_snapshot(store, thread_id, |runtime| {
        runtime.last_context_pressure = pressure.clone();
    })
    .await
}

pub async fn clear_thread_runtime_transients(
    store: &dyn ThreadStorePort,
    thread_id: Uuid,
) -> Result<ThreadRuntimeSnapshot, DatabaseError> {
    mutate_thread_runtime_snapshot(store, thread_id, |runtime| {
        runtime.pending_approval = None;
        runtime.pending_auth = None;
        runtime.post_compaction_context = None;
        runtime.frozen_workspace_prompt = None;
        runtime.frozen_provider_system_prompt = None;
        runtime.prompt_snapshot_hash = None;
        runtime.ephemeral_overlay_hash = None;
        runtime.prompt_segment_order.clear();
        runtime.provider_context_refs.clear();
        if runtime.state == PortableThreadState::AwaitingApproval {
            runtime.state = PortableThreadState::Idle;
        }
    })
    .await
}

/// Truncate durable conversation rows to the active-message watermark
/// before they are replayed into an in-memory thread.
///
/// Rows are ordered oldest-first (as returned by history storage) and the
/// watermark counts how many of the oldest rows are still "active" after
/// `/undo`, `/redo`, or `/clear`. `None` (no watermark recorded yet) keeps
/// every row so pre-existing threads behave exactly as before this field
/// was introduced. This is pure so it can be unit-tested without a store.
pub fn truncate_messages_to_watermark(
    messages: Vec<ThreadMessage>,
    active_message_row_count: Option<i64>,
) -> Vec<ThreadMessage> {
    match active_message_row_count {
        Some(watermark) => {
            let keep = usize::try_from(watermark.max(0)).unwrap_or(usize::MAX);
            messages.into_iter().take(keep).collect()
        }
        None => messages,
    }
}

/// Persist the active-message watermark and a capped undo-stack snapshot in
/// one write. Called after `/undo`, `/redo`, `/clear`, and checkpoint resume
/// mutate the in-memory thread and undo manager, so a restart truncates
/// rehydrated history to what the user actually sees and `/undo` keeps
/// working instead of silently losing its stack.
pub async fn set_active_watermark_and_undo_stack(
    store: &dyn ThreadStorePort,
    thread_id: Uuid,
    active_message_row_count: i64,
    undo_checkpoints: Vec<Checkpoint>,
) -> Result<ThreadRuntimeSnapshot, DatabaseError> {
    mutate_thread_runtime_snapshot(store, thread_id, |runtime| {
        runtime.active_message_row_count = Some(active_message_row_count);
        runtime.undo_checkpoints = undo_checkpoints.clone();
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_correction_prefixes() {
        assert_eq!(
            detect_user_correction_signal("user", "Actually, please use this endpoint."),
            1
        );
        assert_eq!(
            detect_user_correction_signal("user", "No, that's incorrect."),
            1
        );
    }

    #[test]
    fn ignores_non_correction_messages() {
        assert_eq!(
            detect_user_correction_signal("user", "Can you summarize this for me?"),
            0
        );
        assert_eq!(
            detect_user_correction_signal("assistant", "Actually this is fine."),
            0
        );
    }

    #[test]
    fn direct_metadata_updates_mark_new_direct_thread_as_main() {
        let updates = direct_conversation_metadata_updates(&serde_json::json!({}), "web", false);

        assert!(updates.iter().any(|(key, value)| {
            *key == DIRECT_THREAD_ROLE_KEY && value == &serde_json::json!(DIRECT_THREAD_ROLE_MAIN)
        }));
        assert!(
            updates.iter().any(
                |(key, value)| *key == ORIGIN_CHANNEL_KEY && value == &serde_json::json!("web")
            )
        );
        assert!(
            updates
                .iter()
                .any(|(key, value)| *key == SEEN_CHANNELS_KEY
                    && value == &serde_json::json!(["web"]))
        );
    }

    #[test]
    fn direct_metadata_updates_preserve_existing_role_and_seen_channels() {
        let updates = direct_conversation_metadata_updates(
            &serde_json::json!({
                DIRECT_THREAD_ROLE_KEY: "side",
                ORIGIN_CHANNEL_KEY: "imessage",
                SEEN_CHANNELS_KEY: ["imessage", "web"]
            }),
            "web",
            true,
        );

        assert!(
            !updates
                .iter()
                .any(|(key, _)| *key == DIRECT_THREAD_ROLE_KEY || *key == ORIGIN_CHANNEL_KEY)
        );
        assert!(!updates.iter().any(|(key, _)| *key == SEEN_CHANNELS_KEY));
        assert!(
            updates
                .iter()
                .any(|(key, value)| *key == LAST_ACTIVE_CHANNEL_KEY
                    && value == &serde_json::json!("web"))
        );
    }

    #[test]
    fn identifies_primary_direct_thread_metadata() {
        assert!(is_primary_direct_thread_metadata(&serde_json::json!({
            DIRECT_THREAD_ROLE_KEY: DIRECT_THREAD_ROLE_MAIN
        })));
        assert!(is_primary_direct_thread_metadata(&serde_json::json!({
            "thread_type": "assistant"
        })));
        assert!(!is_primary_direct_thread_metadata(&serde_json::json!({
            DIRECT_THREAD_ROLE_KEY: "side"
        })));
    }

    #[test]
    fn primary_direct_candidate_uses_role_or_thread_type() {
        assert!(direct_conversation_candidate_is_primary(
            &serde_json::json!({ DIRECT_THREAD_ROLE_KEY: DIRECT_THREAD_ROLE_MAIN }),
            Some("thread")
        ));
        assert!(direct_conversation_candidate_is_primary(
            &serde_json::json!({ DIRECT_THREAD_ROLE_KEY: "side" }),
            Some("assistant")
        ));
        assert!(!direct_conversation_candidate_is_primary(
            &serde_json::json!({ DIRECT_THREAD_ROLE_KEY: "side" }),
            Some("thread")
        ));
    }

    #[test]
    fn thread_visibility_decision_preserves_owner_fallback_policy() {
        assert_eq!(
            thread_visibility_after_actor_membership("user-1", "actor-1", true),
            ThreadVisibilityDecision::Visible
        );
        assert_eq!(
            thread_visibility_after_actor_membership("user-1", "user-1", false),
            ThreadVisibilityDecision::CheckPrincipalUser
        );
        assert_eq!(
            thread_visibility_after_actor_membership("user-1", "actor-1", false),
            ThreadVisibilityDecision::Hidden
        );
    }

    #[test]
    fn post_compaction_fact_accumulator_decorates_dedupes_and_caps() {
        let mut facts = PostCompactionFactAccumulator::new(3);

        facts.extend_source(
            "Profile",
            vec![
                "Likes Rust".to_string(),
                "likes rust".to_string(),
                "Prefers direct answers".to_string(),
            ],
        );
        assert_eq!(facts.remaining(), 1);

        facts.extend_source(
            "Memory",
            vec![
                "Prefers direct answers".to_string(),
                "Uses web channel".to_string(),
            ],
        );

        assert_eq!(
            facts.into_facts(),
            vec![
                "Profile: Likes Rust",
                "Profile: Prefers direct answers",
                "Memory: Prefers direct answers",
            ]
        );
    }

    #[test]
    fn trajectory_metadata_includes_target_only_when_complete() {
        let session_id = Uuid::new_v4();
        let thread_id = Uuid::new_v4();
        let metadata = trajectory_learning_metadata(thread_id, Some(session_id), Some(3));
        assert_eq!(
            metadata["trajectory_target_id"],
            serde_json::json!(format!("{session_id}:{thread_id}:3"))
        );

        let partial = trajectory_learning_metadata(thread_id, Some(session_id), None);
        assert!(partial.get("trajectory_target_id").is_none());
    }

    #[test]
    fn thread_input_admission_rejects_busy_or_completed_threads() {
        let mut thread = Thread::new(Uuid::new_v4());
        assert_eq!(
            thread_input_admission(&thread),
            ThreadInputAdmission::Accept
        );

        thread.start_turn("work");
        assert_eq!(
            thread_input_admission(&thread),
            ThreadInputAdmission::Reject("Turn in progress. Use /interrupt to cancel.")
        );

        thread.state = ThreadState::Completed;
        assert_eq!(
            thread_input_admission(&thread),
            ThreadInputAdmission::Reject("Thread completed. Use /thread new.")
        );
    }

    #[test]
    fn pending_approval_admission_preserves_mismatched_request() {
        let mut thread = Thread::new(Uuid::new_v4());
        let request_id = Uuid::new_v4();
        let pending = PendingApproval {
            request_id,
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"cmd": "pwd"}),
            description: "inspect cwd".to_string(),
            tool_call_id: "call_1".to_string(),
            context_messages: vec![ChatMessage::user("run pwd")],
            deferred_tool_calls: vec![],
        };
        thread.await_approval(pending);

        assert!(matches!(
            take_pending_approval_matching(&mut thread, Some(Uuid::new_v4())),
            PendingApprovalAdmission::RequestIdMismatch
        ));
        assert_eq!(thread.state, ThreadState::AwaitingApproval);
        assert_eq!(
            thread
                .pending_approval
                .as_ref()
                .map(|pending| pending.request_id),
            Some(request_id)
        );

        let admitted = take_pending_approval_matching(&mut thread, Some(request_id));
        assert!(matches!(admitted, PendingApprovalAdmission::Ready(_)));
        assert!(thread.pending_approval.is_none());
    }

    #[test]
    fn undo_redo_messages_are_policy_owned() {
        assert_eq!(
            undo_redo_message(UndoRedoAction::Undo, &UndoRedoOutcome::NothingAvailable),
            ThreadOperationMessage::Ok("Nothing to undo.".to_string())
        );
        assert_eq!(
            undo_redo_message(
                UndoRedoAction::Undo,
                &UndoRedoOutcome::Restored {
                    turn_number: 4,
                    remaining: 2
                }
            ),
            ThreadOperationMessage::Ok("Undone to turn 4. 2 undo(s) remaining.".to_string())
        );
        assert_eq!(
            undo_redo_message(
                UndoRedoAction::Redo,
                &UndoRedoOutcome::Restored {
                    turn_number: 5,
                    remaining: 0
                }
            ),
            ThreadOperationMessage::Ok("Redone to turn 5.".to_string())
        );
        assert_eq!(
            undo_redo_message(UndoRedoAction::Redo, &UndoRedoOutcome::Failed),
            ThreadOperationMessage::Error("Redo failed.")
        );
    }

    #[test]
    fn auth_helpers_clear_reenter_and_format_status_messages() {
        let mut thread = Thread::new(Uuid::new_v4());
        let pending = crate::session::PendingAuth {
            extension_name: "github".to_string(),
            auth_mode: crate::session::PendingAuthMode::ExternalOAuth,
        };
        reenter_pending_auth(&mut thread, &pending);
        assert!(thread.pending_auth.is_some());
        assert!(clear_pending_auth(&mut thread));
        assert!(thread.pending_auth.is_none());
        assert!(!clear_pending_auth(&mut thread));

        assert_eq!(
            auth_required_status_mode(None, crate::session::PendingAuthMode::ExternalOAuth),
            "oauth"
        );
        assert_eq!(auth_required_status(None), "awaiting_token");
        assert_eq!(
            auth_activation_success_message("github", &["issues".to_string(), "prs".to_string()]),
            "github authenticated and activated (2 tools loaded).\n\nTools: issues, prs"
        );
        assert_eq!(
            auth_activation_failed_message("github", "boom"),
            "github authenticated successfully, but activation failed: boom. Try activating manually."
        );
        assert_eq!(
            invalid_auth_token_message(None),
            "Invalid token. Please try again."
        );
        assert_eq!(
            auth_failed_message("github", "network"),
            "Authentication failed for github: network"
        );
    }

    #[test]
    fn runtime_snapshot_preserves_existing_context_and_prompt_fields() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("work");
        let existing = ThreadRuntimeSnapshot {
            post_compaction_context: Some("Recent compacted facts".to_string()),
            frozen_workspace_prompt: Some("workspace".to_string()),
            frozen_provider_system_prompt: Some("provider".to_string()),
            prompt_snapshot_hash: Some("hash".to_string()),
            ephemeral_overlay_hash: Some("overlay".to_string()),
            prompt_segment_order: vec!["base".to_string(), "workspace".to_string()],
            provider_context_refs: vec!["ctx-1".to_string()],
            last_context_pressure: Some(serde_json::json!({"usage": 0.8})),
            auto_approved_tools: vec!["shell".to_string()],
            ..Default::default()
        };

        let snapshot = runtime_snapshot_for_persistence(
            &thread,
            None,
            None,
            None,
            Vec::new(),
            Some(&existing),
        );

        assert_eq!(
            snapshot.post_compaction_context.as_deref(),
            Some("Recent compacted facts")
        );
        assert_eq!(snapshot.prompt_snapshot_hash.as_deref(), Some("hash"));
        assert_eq!(snapshot.prompt_segment_order, ["base", "workspace"]);
        assert_eq!(snapshot.provider_context_refs, ["ctx-1"]);
        assert_eq!(snapshot.auto_approved_tools, ["shell"]);
        assert_eq!(
            snapshot.last_context_pressure,
            existing.last_context_pressure
        );
    }

    #[test]
    fn start_user_turn_checkpoints_and_applies_visibility_metadata() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("previous");
        thread.complete_turn("done");
        let mut undo = UndoManager::new();

        let messages = start_user_turn(
            &mut thread,
            &mut undo,
            "hidden prompt",
            &serde_json::json!({"hide_from_webui_chat": true}),
        );

        assert_eq!(undo.undo_count(), 1);
        assert_eq!(messages.len(), 3);
        assert_eq!(thread.turns.len(), 2);
        assert!(thread.turns[1].hide_user_input_from_ui);
    }

    #[test]
    fn interrupt_thread_only_changes_active_threads() {
        let mut thread = Thread::new(Uuid::new_v4());
        assert!(!interrupt_thread(&mut thread));

        thread.start_turn("work");
        assert!(interrupt_thread(&mut thread));
        assert_eq!(thread.state, ThreadState::Interrupted);
    }

    #[test]
    fn clear_thread_resets_transient_state() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("work");
        thread.pending_auth = Some(crate::session::PendingAuth {
            extension_name: "github".to_string(),
            auth_mode: crate::session::PendingAuthMode::ManualToken,
        });

        clear_thread(&mut thread);

        assert!(thread.turns.is_empty());
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.pending_auth.is_none());
    }

    struct MemoryThreadStore {
        runtime: tokio::sync::Mutex<Option<ThreadRuntimeSnapshot>>,
    }

    impl MemoryThreadStore {
        fn new(runtime: Option<ThreadRuntimeSnapshot>) -> Self {
            Self {
                runtime: tokio::sync::Mutex::new(runtime),
            }
        }
    }

    #[async_trait::async_trait]
    impl ThreadStorePort for MemoryThreadStore {
        async fn ensure_thread(
            &self,
            _thread_id: Uuid,
            _channel: &str,
            _user_id: &str,
            _external_thread_id: Option<&str>,
        ) -> Result<(), DatabaseError> {
            Ok(())
        }

        async fn load_thread_runtime(
            &self,
            _thread_id: Uuid,
        ) -> Result<Option<ThreadRuntimeSnapshot>, DatabaseError> {
            Ok(self.runtime.lock().await.clone())
        }

        async fn save_thread_runtime(
            &self,
            _thread_id: Uuid,
            runtime: &ThreadRuntimeSnapshot,
        ) -> Result<(), DatabaseError> {
            *self.runtime.lock().await = Some(runtime.clone());
            Ok(())
        }

        async fn append_thread_message(
            &self,
            _thread_id: Uuid,
            _role: &str,
            _content: &str,
            _attribution: Option<&serde_json::Value>,
        ) -> Result<Uuid, DatabaseError> {
            Ok(Uuid::new_v4())
        }

        async fn list_thread_messages(
            &self,
            _thread_id: Uuid,
            _before: Option<chrono::DateTime<chrono::Utc>>,
            _limit: i64,
        ) -> Result<Vec<crate::ports::ThreadMessage>, DatabaseError> {
            Ok(Vec::new())
        }

        async fn list_threads_for_recall(
            &self,
            _scope: &crate::ports::AgentScope,
            _include_group_history: bool,
            _limit: i64,
        ) -> Result<Vec<crate::ports::ThreadSummary>, DatabaseError> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn clears_runtime_transients_via_thread_store_port() {
        let thread_id = Uuid::new_v4();
        let store = MemoryThreadStore::new(Some(ThreadRuntimeSnapshot {
            post_compaction_context: Some("ctx".to_string()),
            frozen_workspace_prompt: Some("workspace".to_string()),
            prompt_snapshot_hash: Some("hash".to_string()),
            prompt_segment_order: vec!["a".to_string()],
            provider_context_refs: vec!["ref".to_string()],
            ..Default::default()
        }));

        let runtime = clear_thread_runtime_transients(&store, thread_id)
            .await
            .unwrap();

        assert!(runtime.post_compaction_context.is_none());
        assert!(runtime.frozen_workspace_prompt.is_none());
        assert!(runtime.prompt_snapshot_hash.is_none());
        assert!(runtime.prompt_segment_order.is_empty());
        assert!(runtime.provider_context_refs.is_empty());
    }

    fn sample_thread_message(role: &str, content: &str) -> ThreadMessage {
        ThreadMessage {
            id: Uuid::new_v4(),
            conversation_id: Uuid::new_v4(),
            role: role.to_string(),
            content: content.to_string(),
            actor_id: None,
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn truncate_messages_to_watermark_keeps_oldest_n_rows() {
        let messages = vec![
            sample_thread_message("user", "first"),
            sample_thread_message("assistant", "first reply"),
            sample_thread_message("user", "second"),
            sample_thread_message("assistant", "second reply"),
        ];

        let truncated = truncate_messages_to_watermark(messages, Some(2));
        assert_eq!(truncated.len(), 2);
        assert_eq!(truncated[0].content, "first");
        assert_eq!(truncated[1].content, "first reply");
    }

    #[test]
    fn truncate_messages_to_watermark_zero_clears_history() {
        let messages = vec![
            sample_thread_message("user", "first"),
            sample_thread_message("assistant", "first reply"),
        ];

        let truncated = truncate_messages_to_watermark(messages, Some(0));
        assert!(truncated.is_empty());
    }

    #[test]
    fn truncate_messages_to_watermark_none_keeps_all_rows() {
        let messages = vec![
            sample_thread_message("user", "first"),
            sample_thread_message("assistant", "first reply"),
        ];

        let truncated = truncate_messages_to_watermark(messages.clone(), None);
        assert_eq!(truncated.len(), messages.len());
    }

    #[test]
    fn truncate_messages_to_watermark_beyond_len_is_noop() {
        let messages = vec![sample_thread_message("user", "only")];

        let truncated = truncate_messages_to_watermark(messages.clone(), Some(50));
        assert_eq!(truncated.len(), 1);
    }

    #[tokio::test]
    async fn set_active_watermark_and_undo_stack_persists_both_fields() {
        let thread_id = Uuid::new_v4();
        let store = MemoryThreadStore::new(None);
        let checkpoints = vec![Checkpoint::new(1, vec![], "Turn 1")];

        let runtime =
            set_active_watermark_and_undo_stack(&store, thread_id, 2, checkpoints.clone())
                .await
                .unwrap();

        assert_eq!(runtime.active_message_row_count, Some(2));
        assert_eq!(runtime.undo_checkpoints.len(), 1);
        assert_eq!(runtime.undo_checkpoints[0].turn_number, 1);
    }

    #[tokio::test]
    async fn updates_context_pressure_via_thread_store_port() {
        let thread_id = Uuid::new_v4();
        let store = MemoryThreadStore::new(None);
        let pressure = serde_json::json!({"level":"high"});

        set_last_context_pressure(&store, thread_id, Some(pressure.clone()))
            .await
            .unwrap();

        assert_eq!(
            load_last_context_pressure(&store, thread_id).await.unwrap(),
            Some(pressure)
        );
    }
}
