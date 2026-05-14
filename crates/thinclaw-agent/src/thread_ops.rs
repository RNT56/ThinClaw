//! Root-independent thread operation helpers.

use uuid::Uuid;

use thinclaw_llm_core::ChatMessage;

use crate::ports::{PortableThreadState, ThreadRuntimeSnapshot, ThreadStorePort};
use crate::session::{
    PendingApproval, PendingAuthMode, Thread, ThreadState, message_hides_user_input_in_main_chat,
};
use crate::undo::UndoManager;
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
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoRedoOutcome {
    Restored {
        turn_number: usize,
        remaining: usize,
    },
    NothingAvailable,
    Failed,
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
