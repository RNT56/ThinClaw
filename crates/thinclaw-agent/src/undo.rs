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
        self.undo_stack.push_back(checkpoint);
        while self.undo_stack.len() > self.max_checkpoints {
            self.undo_stack.pop_front();
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
        self.redo_stack.push(current);

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
        let skip = self.undo_stack.len().saturating_sub(max);
        self.undo_stack.iter().skip(skip).cloned().collect()
    }

    /// Rebuild an undo manager from a persisted (already capped) checkpoint
    /// list, preserving this manager's configured checkpoint limit.
    pub fn restore_from_checkpoints(&mut self, checkpoints: Vec<Checkpoint>) {
        self.undo_stack = checkpoints.into_iter().collect();
        while self.undo_stack.len() > self.max_checkpoints {
            self.undo_stack.pop_front();
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
}

impl Default for UndoManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_creation() {
        let mut manager = UndoManager::new();

        manager.checkpoint(0, vec![], "Initial state");
        manager.checkpoint(1, vec![ChatMessage::user("Hello")], "Turn 1");

        assert_eq!(manager.undo_count(), 2);
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
