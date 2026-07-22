//! Undo system with checkpoints.
//!
//! Provides the ability to roll back the conversation state to a previous point.
//! Checkpoints are created automatically at the start of each turn.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use thinclaw_llm_core::ChatMessage;

/// Maximum number of checkpoints to keep by default.
const DEFAULT_MAX_CHECKPOINTS: usize = 20;
/// Hard cap for full-history snapshots retained in memory per thread.
const MAX_IN_MEMORY_CHECKPOINT_BYTES: usize = 8 * 1024 * 1024;
/// Hard cap for checkpoints embedded in the durable runtime JSON envelope.
const MAX_PERSISTED_CHECKPOINT_BYTES: usize = 2 * 1024 * 1024;

/// Maximum number of checkpoints persisted into the durable thread runtime
/// envelope. Checkpoints carry full message history, so persistence keeps
/// only the newest few rather than the full in-memory `max_checkpoints`.
pub const MAX_PERSISTED_CHECKPOINTS: usize = 5;

/// A snapshot of conversation state at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Unique checkpoint ID.
    pub id: Uuid,
    /// Turn number this checkpoint was created at.
    pub turn_number: usize,
    /// Snapshot of messages at this point.
    pub messages: Vec<ChatMessage>,
    /// Description of what happened at this checkpoint.
    pub description: String,
}

impl Checkpoint {
    /// Create a new checkpoint.
    pub fn new(
        turn_number: usize,
        messages: Vec<ChatMessage>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            turn_number,
            messages,
            description: description.into(),
        }
    }
}

/// Manager for undo/redo functionality.
///
/// Each undo/redo operation pops from one stack and pushes the current state
/// onto the other, so `undo_count() + redo_count()` stays constant across
/// undo/redo cycles (only `checkpoint()` and `clear()` change the total).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoManager {
    /// Stack of past checkpoints (for undo).
    undo_stack: VecDeque<Checkpoint>,
    /// Stack of future checkpoints (for redo).
    redo_stack: Vec<Checkpoint>,
    /// Maximum checkpoints to keep.
    max_checkpoints: usize,
}

impl UndoManager {
    /// Create a new undo manager.
    pub fn new() -> Self {
        Self {
            undo_stack: VecDeque::new(),
            redo_stack: Vec::new(),
            max_checkpoints: DEFAULT_MAX_CHECKPOINTS,
        }
    }

    /// Create with a custom checkpoint limit.
    #[cfg(test)]
    pub fn with_max_checkpoints(mut self, max: usize) -> Self {
        self.max_checkpoints = max;
        self
    }

    /// Push a checkpoint onto the undo stack, trimming oldest entries if over limit.
    fn push_undo(&mut self, checkpoint: Checkpoint) {
        let checkpoint = checkpoint_for_storage(checkpoint);
        if checkpoint_bytes(&checkpoint) > MAX_IN_MEMORY_CHECKPOINT_BYTES {
            tracing::warn!(
                turn = checkpoint.turn_number,
                "Skipping oversized conversation undo checkpoint"
            );
            return;
        }
        self.undo_stack.push_back(checkpoint);
        while self.undo_stack.len() > self.max_checkpoints
            || (self.undo_stack.len() > 1
                && checkpoint_stack_bytes(&self.undo_stack) > MAX_IN_MEMORY_CHECKPOINT_BYTES)
        {
            self.undo_stack.pop_front();
        }
    }

    fn push_redo(&mut self, checkpoint: Checkpoint) {
        let checkpoint = checkpoint_for_storage(checkpoint);
        if checkpoint_bytes(&checkpoint) > MAX_IN_MEMORY_CHECKPOINT_BYTES {
            tracing::warn!(
                turn = checkpoint.turn_number,
                "Skipping oversized conversation redo checkpoint"
            );
            return;
        }
        self.redo_stack.push(checkpoint);
        while self.redo_stack.len() > self.max_checkpoints
            || (self.redo_stack.len() > 1
                && checkpoint_slice_bytes(&self.redo_stack) > MAX_IN_MEMORY_CHECKPOINT_BYTES)
        {
            self.redo_stack.remove(0);
        }
    }

    /// Create a checkpoint at the current state.
    ///
    /// This clears the redo stack since we're creating a new history branch.
    pub fn checkpoint(
        &mut self,
        turn_number: usize,
        messages: Vec<ChatMessage>,
        description: impl Into<String>,
    ) {
        // Clear redo stack (new branch of history)
        self.redo_stack.clear();

        let checkpoint = Checkpoint::new(turn_number, messages, description);
        self.push_undo(checkpoint);
    }

    /// Undo: pop the last checkpoint and return it.
    ///
    /// Saves the current state to the redo stack and pops the most recent
    /// checkpoint from the undo stack so that repeated undos walk backwards
    /// through history.
    ///
    /// Takes ownership of `current_messages`; callers must clone first if
    /// they need to retain a copy.
    pub fn undo(
        &mut self,
        current_turn: usize,
        current_messages: Vec<ChatMessage>,
    ) -> Option<Checkpoint> {
        if self.undo_stack.is_empty() {
            return None;
        }

        // Save current state to redo stack
        let current = Checkpoint::new(
            current_turn,
            current_messages,
            format!("Turn {}", current_turn),
        );
        self.push_redo(current);

        // Pop and return the most recent checkpoint
        self.undo_stack.pop_back()
    }

    /// Pop the last checkpoint from the undo stack.
    #[cfg(test)]
    pub fn pop_undo(&mut self) -> Option<Checkpoint> {
        self.undo_stack.pop_back()
    }

    /// Redo: restore a previously undone state.
    ///
    /// Saves the current state to the undo stack and pops the most recent
    /// checkpoint from the redo stack.
    ///
    /// Takes ownership of `current_messages`; callers must clone first if
    /// they need to retain a copy.
    pub fn redo(
        &mut self,
        current_turn: usize,
        current_messages: Vec<ChatMessage>,
    ) -> Option<Checkpoint> {
        if self.redo_stack.is_empty() {
            return None;
        }

        // Save current state to undo stack
        let current = Checkpoint::new(
            current_turn,
            current_messages,
            format!("Turn {}", current_turn),
        );
        self.push_undo(current);

        self.redo_stack.pop()
    }

    /// Check if undo is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if redo is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Get the number of undo steps available.
    pub fn undo_count(&self) -> usize {
        self.undo_stack.len()
    }

    /// Get the number of redo steps available.
    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }

    /// Get a checkpoint by ID.
    #[cfg(test)]
    pub fn get_checkpoint(&self, id: Uuid) -> Option<&Checkpoint> {
        self.undo_stack
            .iter()
            .find(|c| c.id == id)
            .or_else(|| self.redo_stack.iter().find(|c| c.id == id))
    }

    /// List all available checkpoints (for UI display).
    #[cfg(test)]
    pub fn list_checkpoints(&self) -> Vec<&Checkpoint> {
        self.undo_stack.iter().collect()
    }

    /// Clear all checkpoints.
    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Snapshot the newest `max` undo checkpoints (oldest to newest) for
    /// durable persistence. The redo stack is intentionally not persisted:
    /// it represents speculative future state that is safe to drop across a
    /// restart, whereas the undo stack is what `/undo` needs to keep working.
    pub fn persisted_checkpoints(&self, max: usize) -> Vec<Checkpoint> {
        let mut bytes = 0usize;
        let mut newest_first = Vec::new();
        for checkpoint in self.undo_stack.iter().rev() {
            if newest_first.len() >= max {
                break;
            }
            let checkpoint = checkpoint_for_storage(checkpoint.clone());
            let checkpoint_bytes = checkpoint_bytes(&checkpoint);
            if checkpoint_bytes > MAX_PERSISTED_CHECKPOINT_BYTES
                || bytes.saturating_add(checkpoint_bytes) > MAX_PERSISTED_CHECKPOINT_BYTES
            {
                continue;
            }
            bytes = bytes.saturating_add(checkpoint_bytes);
            newest_first.push(checkpoint);
        }
        newest_first.reverse();
        newest_first
    }

    /// Rebuild an undo manager from a persisted (already capped) checkpoint
    /// list, preserving this manager's configured checkpoint limit.
    pub fn restore_from_checkpoints(&mut self, checkpoints: Vec<Checkpoint>) {
        // Re-sanitize on read as well as write so runtime envelopes created by
        // older versions cannot reintroduce raw tool arguments into memory and
        // then be copied into a later snapshot.
        self.undo_stack.clear();
        let mut bytes = 0usize;
        for checkpoint in checkpoints.into_iter().rev() {
            if self.undo_stack.len() >= self.max_checkpoints {
                break;
            }
            let checkpoint = checkpoint_for_storage(checkpoint);
            let checkpoint_bytes = checkpoint_bytes(&checkpoint);
            if checkpoint_bytes > MAX_PERSISTED_CHECKPOINT_BYTES
                || bytes.saturating_add(checkpoint_bytes) > MAX_PERSISTED_CHECKPOINT_BYTES
            {
                continue;
            }
            bytes = bytes.saturating_add(checkpoint_bytes);
            self.undo_stack.push_front(checkpoint);
        }
        self.redo_stack.clear();
    }

    /// Restore to a specific checkpoint by ID.
    ///
    /// This invalidates all checkpoints after this one.
    pub fn restore(&mut self, checkpoint_id: Uuid) -> Option<Checkpoint> {
        // Find the checkpoint position
        let pos = self.undo_stack.iter().position(|c| c.id == checkpoint_id)?;

        // Clear redo stack
        self.redo_stack.clear();

        // Remove all checkpoints after this one
        while self.undo_stack.len() > pos + 1 {
            self.undo_stack.pop_back();
        }

        // Pop and return the target checkpoint
        self.undo_stack.pop_back()
    }

    /// Restore conversation state to a specific turn number.
    ///
    /// Finds the checkpoint captured at the start of `turn_number` and restores
    /// to it with the same semantics as [`UndoManager::restore`] (clears the
    /// redo stack and discards every later checkpoint). Returns `None` when no
    /// checkpoint exists for that turn (e.g. it aged out of the ring buffer).
    pub fn restore_to_turn(&mut self, turn_number: usize) -> Option<Checkpoint> {
        let id = self
            .undo_stack
            .iter()
            .find(|c| c.turn_number == turn_number)
            .map(|c| c.id)?;
        self.restore(id)
    }

    /// The `(turn_number, description)` of every available conversation
    /// checkpoint, oldest first — used by `/rewind list` to show rewind targets.
    pub fn checkpoint_turns(&self) -> Vec<(usize, String)> {
        self.undo_stack
            .iter()
            .map(|c| (c.turn_number, c.description.clone()))
            .collect()
    }

    /// Whether a conversation checkpoint exists for `turn_number`.
    pub fn has_turn(&self, turn_number: usize) -> bool {
        self.undo_stack.iter().any(|c| c.turn_number == turn_number)
    }
}

fn checkpoint_bytes(checkpoint: &Checkpoint) -> usize {
    serde_json::to_vec(checkpoint)
        .map(|encoded| encoded.len())
        .unwrap_or(usize::MAX)
}

fn checkpoint_stack_bytes(checkpoints: &VecDeque<Checkpoint>) -> usize {
    checkpoints.iter().fold(0usize, |total, checkpoint| {
        total.saturating_add(checkpoint_bytes(checkpoint))
    })
}

fn checkpoint_slice_bytes(checkpoints: &[Checkpoint]) -> usize {
    checkpoints.iter().fold(0usize, |total, checkpoint| {
        total.saturating_add(checkpoint_bytes(checkpoint))
    })
}

fn checkpoint_for_storage(mut checkpoint: Checkpoint) -> Checkpoint {
    for message in &mut checkpoint.messages {
        if let Some(tool_calls) = message.tool_calls.as_mut() {
            for tool_call in tool_calls {
                tool_call.arguments =
                    crate::session::summarized_tool_parameters(&tool_call.arguments);
            }
        }
    }
    checkpoint
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thinclaw_llm_core::ToolCall;

    #[test]
    fn test_checkpoint_creation() {
        let mut manager = UndoManager::new();

        manager.checkpoint(0, vec![], "Initial state");
        manager.checkpoint(1, vec![ChatMessage::user("Hello")], "Turn 1");

        assert_eq!(manager.undo_count(), 2);
    }

    #[test]
    fn test_restore_to_turn_and_listing() {
        let mut manager = UndoManager::new();
        manager.checkpoint(0, vec![], "Before turn 0");
        manager.checkpoint(1, vec![ChatMessage::user("a")], "Before turn 1");
        manager.checkpoint(2, vec![ChatMessage::user("b")], "Before turn 2");

        assert!(manager.has_turn(1));
        assert!(!manager.has_turn(9));
        let turns: Vec<usize> = manager
            .checkpoint_turns()
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        assert_eq!(turns, vec![0, 1, 2]);

        // Rewind to turn 1: returns that checkpoint and discards later ones.
        let restored = manager.restore_to_turn(1).expect("turn 1 present");
        assert_eq!(restored.turn_number, 1);
        assert!(!manager.has_turn(2), "later checkpoints are dropped");
        assert!(!manager.can_redo(), "redo stack cleared on restore");

        // Unknown turn is a no-op returning None.
        assert!(manager.restore_to_turn(42).is_none());
    }

    #[test]
    fn test_undo_redo() {
        let mut manager = UndoManager::new();

        manager.checkpoint(0, vec![], "Turn 0");
        manager.checkpoint(1, vec![ChatMessage::user("Hello")], "Turn 1");

        assert!(manager.can_undo());
        assert!(!manager.can_redo());

        // Undo - returns owned Checkpoint now
        let current = vec![ChatMessage::user("Hello"), ChatMessage::assistant("Hi")];
        let checkpoint = manager.undo(2, current);
        assert!(checkpoint.is_some());
        let checkpoint = checkpoint.unwrap();
        assert_eq!(checkpoint.turn_number, 1);
        assert!(manager.can_redo());

        // Redo - now requires current state parameters
        let restored = manager.redo(checkpoint.turn_number, checkpoint.messages);
        assert!(restored.is_some());
    }

    #[test]
    fn test_max_checkpoints() {
        let mut manager = UndoManager::new().with_max_checkpoints(3);

        for i in 0..5 {
            manager.checkpoint(i, vec![], format!("Turn {}", i));
        }

        assert_eq!(manager.undo_count(), 3);
    }

    #[test]
    fn test_restore_to_checkpoint() {
        let mut manager = UndoManager::new();

        manager.checkpoint(0, vec![], "Turn 0");
        let checkpoint_id = manager.undo_stack.back().unwrap().id;
        manager.checkpoint(1, vec![], "Turn 1");
        manager.checkpoint(2, vec![], "Turn 2");

        let restored = manager.restore(checkpoint_id);
        assert!(restored.is_some());
        assert_eq!(manager.undo_count(), 0);
    }

    #[test]
    fn test_repeated_undo_advances_through_stack() {
        let mut manager = UndoManager::new();

        // Create 3 checkpoints at turns 0, 1, 2
        manager.checkpoint(0, vec![], "Turn 0");
        manager.checkpoint(1, vec![ChatMessage::user("msg1")], "Turn 1");
        manager.checkpoint(2, vec![ChatMessage::user("msg2")], "Turn 2");
        assert_eq!(manager.undo_count(), 3);

        // First undo: should return turn 2 checkpoint, stack shrinks to 2
        let cp1 = manager
            .undo(3, vec![ChatMessage::user("msg3")])
            .expect("first undo should succeed");
        assert_eq!(cp1.turn_number, 2);
        assert_eq!(manager.undo_count(), 2);

        // Second undo: should return turn 1 checkpoint (different!), stack shrinks to 1
        let cp2 = manager
            .undo(cp1.turn_number, cp1.messages)
            .expect("second undo should succeed");
        assert_eq!(cp2.turn_number, 1);
        assert_eq!(manager.undo_count(), 1);

        // Verify we walked backwards through distinct checkpoints
        assert_ne!(cp1.turn_number, cp2.turn_number);
    }

    #[test]
    fn test_undo_redo_cycle_preserves_state() {
        let mut manager = UndoManager::new();

        let msgs_t0: Vec<ChatMessage> = vec![];
        let msgs_t1 = vec![ChatMessage::user("hello")];
        let msgs_t2 = vec![ChatMessage::user("hello"), ChatMessage::assistant("hi")];

        manager.checkpoint(0, msgs_t0, "Turn 0");
        manager.checkpoint(1, msgs_t1, "Turn 1");

        // Undo from turn 2 -> get turn 1 checkpoint
        let cp_undo1 = manager
            .undo(2, msgs_t2.clone())
            .expect("undo should succeed");
        assert_eq!(cp_undo1.turn_number, 1);

        // Redo from turn 1 -> get turn 2 state back
        let cp_redo = manager
            .redo(cp_undo1.turn_number, cp_undo1.messages)
            .expect("redo should succeed");
        assert_eq!(cp_redo.turn_number, 2);
        assert_eq!(cp_redo.messages.len(), 2);

        // Undo again from turn 2 -> should go back to turn 1 again
        let cp_undo2 = manager
            .undo(cp_redo.turn_number, cp_redo.messages)
            .expect("second undo should succeed");
        assert_eq!(cp_undo2.turn_number, 1);
    }

    #[test]
    fn test_persisted_checkpoints_caps_to_newest() {
        let mut manager = UndoManager::new();
        for i in 0..8 {
            manager.checkpoint(i, vec![], format!("Turn {}", i));
        }
        assert_eq!(manager.undo_count(), 8);

        let persisted = manager.persisted_checkpoints(MAX_PERSISTED_CHECKPOINTS);
        assert_eq!(persisted.len(), MAX_PERSISTED_CHECKPOINTS);
        // Newest checkpoints (turns 3..=7) should be kept, oldest-first order preserved.
        let turn_numbers: Vec<usize> = persisted.iter().map(|c| c.turn_number).collect();
        assert_eq!(turn_numbers, vec![3, 4, 5, 6, 7]);
    }

    #[test]
    fn test_persisted_checkpoints_smaller_than_cap_returns_all() {
        let mut manager = UndoManager::new();
        manager.checkpoint(0, vec![], "Turn 0");
        manager.checkpoint(1, vec![], "Turn 1");

        let persisted = manager.persisted_checkpoints(MAX_PERSISTED_CHECKPOINTS);
        assert_eq!(persisted.len(), 2);
    }

    #[test]
    fn test_undo_stack_round_trips_through_serialization_with_cap() {
        let mut manager = UndoManager::new();
        for i in 0..8 {
            manager.checkpoint(
                i,
                vec![ChatMessage::user(format!("msg-{i}"))],
                format!("Turn {}", i),
            );
        }

        let persisted = manager.persisted_checkpoints(MAX_PERSISTED_CHECKPOINTS);
        let json = serde_json::to_string(&persisted).expect("serialize checkpoints");
        let decoded: Vec<Checkpoint> =
            serde_json::from_str(&json).expect("deserialize checkpoints");
        assert_eq!(decoded.len(), MAX_PERSISTED_CHECKPOINTS);

        let mut restored = UndoManager::new();
        restored.restore_from_checkpoints(decoded);
        assert_eq!(restored.undo_count(), MAX_PERSISTED_CHECKPOINTS);
        assert!(!restored.can_redo());

        // The restored stack should walk backwards through the newest
        // checkpoints, most recent first.
        let cp = restored
            .undo(8, vec![])
            .expect("undo should succeed after restore");
        assert_eq!(cp.turn_number, 7);
    }

    #[test]
    fn test_restore_from_checkpoints_respects_max_checkpoints_limit() {
        let mut manager = UndoManager::new().with_max_checkpoints(3);
        let checkpoints: Vec<Checkpoint> = (0..5)
            .map(|i| Checkpoint::new(i, vec![], format!("Turn {}", i)))
            .collect();

        manager.restore_from_checkpoints(checkpoints);

        assert_eq!(manager.undo_count(), 3);
    }

    #[test]
    fn persisted_checkpoints_redact_tool_argument_values() {
        let secret = "super-secret-token";
        let tool_message = ChatMessage::assistant_with_tool_calls(
            None,
            vec![ToolCall {
                id: "call-1".to_string(),
                name: "http".to_string(),
                arguments: serde_json::json!({
                    "authorization": secret,
                    "url": "https://example.test/private"
                }),
            }],
        );
        let mut manager = UndoManager::new();
        manager.checkpoint(1, vec![tool_message], "secret-bearing tool call");

        let persisted = manager.persisted_checkpoints(MAX_PERSISTED_CHECKPOINTS);
        let encoded = serde_json::to_string(&persisted).expect("serialize checkpoints");

        assert!(!encoded.contains(secret));
        assert!(!encoded.contains("https://example.test/private"));
        let summary = &persisted[0].messages[0].tool_calls.as_ref().unwrap()[0].arguments;
        assert_eq!(summary["_thinclaw_parameter_values_redacted"], true);
        assert_eq!(summary["keys"], serde_json::json!(["authorization", "url"]));
    }

    #[test]
    fn restoring_legacy_checkpoints_sanitizes_raw_tool_arguments() {
        let checkpoint = Checkpoint::new(
            1,
            vec![ChatMessage::assistant_with_tool_calls(
                None,
                vec![ToolCall {
                    id: "call-legacy".to_string(),
                    name: "form_input".to_string(),
                    arguments: serde_json::json!({"password": "legacy-secret"}),
                }],
            )],
            "legacy",
        );
        let mut manager = UndoManager::new();

        manager.restore_from_checkpoints(vec![checkpoint]);

        let restored = manager.list_checkpoints()[0];
        let arguments = &restored.messages[0].tool_calls.as_ref().unwrap()[0].arguments;
        assert_eq!(arguments["_thinclaw_parameter_values_redacted"], true);
        assert!(!arguments.to_string().contains("legacy-secret"));
    }

    #[test]
    fn persisted_checkpoint_byte_budget_skips_oversized_entries() {
        let mut manager = UndoManager::new();
        manager.checkpoint(
            1,
            vec![ChatMessage::user(
                "x".repeat(MAX_PERSISTED_CHECKPOINT_BYTES + 1),
            )],
            "oversized",
        );
        manager.checkpoint(2, vec![ChatMessage::user("small")], "small");

        let persisted = manager.persisted_checkpoints(MAX_PERSISTED_CHECKPOINTS);

        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].turn_number, 2);
        assert!(checkpoint_slice_bytes(&persisted) <= MAX_PERSISTED_CHECKPOINT_BYTES);
    }

    #[test]
    fn restore_skips_oversized_persisted_entries() {
        let oversized = Checkpoint::new(
            1,
            vec![ChatMessage::user(
                "x".repeat(MAX_PERSISTED_CHECKPOINT_BYTES + 1),
            )],
            "oversized",
        );
        let small = Checkpoint::new(2, vec![ChatMessage::user("small")], "small");
        let mut manager = UndoManager::new();

        manager.restore_from_checkpoints(vec![oversized, small]);

        assert_eq!(manager.undo_count(), 1);
        assert_eq!(manager.list_checkpoints()[0].turn_number, 2);
        assert!(checkpoint_stack_bytes(&manager.undo_stack) <= MAX_PERSISTED_CHECKPOINT_BYTES);
    }

    #[test]
    fn test_undo_redo_stack_sizes_consistent() {
        let mut manager = UndoManager::new();

        manager.checkpoint(0, vec![], "Turn 0");
        manager.checkpoint(1, vec![ChatMessage::user("a")], "Turn 1");
        manager.checkpoint(2, vec![ChatMessage::user("b")], "Turn 2");

        // Start: undo=3, redo=0, total=3
        let total = manager.undo_count() + manager.redo_count();
        assert_eq!(total, 3);

        // After undo: total should still be 3 (one moved from undo to redo,
        // plus the current state pushed to redo)
        // Actually: undo pops one (3->2), pushes current to redo (0->1), total=3
        let cp = manager.undo(3, vec![]).unwrap();
        assert_eq!(manager.undo_count() + manager.redo_count(), 3);

        // After redo: redo pops one (1->0), pushes current to undo (2->3), total=3
        let cp2 = manager.redo(cp.turn_number, cp.messages).unwrap();
        assert_eq!(manager.undo_count() + manager.redo_count(), 3);

        // After another undo: same invariant
        let _cp3 = manager.undo(cp2.turn_number, cp2.messages).unwrap();
        assert_eq!(manager.undo_count() + manager.redo_count(), 3);
    }
}
