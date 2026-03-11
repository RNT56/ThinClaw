//! Reaction state machine with watchdog + debounce.
//!
//! Manages the lifecycle of acknowledgement reactions on messages:
//! 1. Receive message → add ack reaction
//! 2. Processing starts → optionally swap to "thinking" reaction
//! 3. Processing completes → swap to "done" reaction (or remove ack)
//!
//! Includes debouncing to avoid Telegram/Discord rate limits on
//! rapid reaction changes, and a watchdog to clean up stale reactions.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// State of a reaction on a specific message.
#[derive(Debug, Clone, PartialEq)]
pub enum ReactionState {
    /// Acknowledge receipt.
    Acknowledged,
    /// Agent is processing.
    Processing,
    /// Processing complete.
    Done,
    /// Error occurred.
    Error,
    /// Reaction has been removed.
    Removed,
}

/// A tracked reaction entry.
#[derive(Debug, Clone)]
struct ReactionEntry {
    /// Current state.
    state: ReactionState,
    /// When the current state was entered.
    state_changed_at: Instant,
    /// When the last API call was made.
    last_api_call: Option<Instant>,
    /// How many API calls have been made for this reaction.
    api_call_count: u32,
}

/// Reaction state machine with debouncing.
pub struct ReactionStateMachine {
    /// Active reactions keyed by (channel, message_id).
    reactions: HashMap<(String, String), ReactionEntry>,
    /// Minimum time between API calls per reaction.
    debounce: Duration,
    /// Maximum age before a reaction is considered stale.
    watchdog_timeout: Duration,
}

impl ReactionStateMachine {
    /// Create a new reaction state machine.
    pub fn new() -> Self {
        Self {
            reactions: HashMap::new(),
            debounce: Duration::from_millis(500),
            watchdog_timeout: Duration::from_secs(300), // 5 minutes
        }
    }

    /// Create with custom timing.
    pub fn with_timing(debounce_ms: u64, watchdog_secs: u64) -> Self {
        Self {
            reactions: HashMap::new(),
            debounce: Duration::from_millis(debounce_ms),
            watchdog_timeout: Duration::from_secs(watchdog_secs),
        }
    }

    /// Transition a reaction to a new state. Returns true if the state
    /// change should be applied (i.e., debounce period has elapsed).
    pub fn transition(
        &mut self,
        channel: &str,
        message_id: &str,
        new_state: ReactionState,
    ) -> bool {
        let key = (channel.to_string(), message_id.to_string());
        let now = Instant::now();

        if let Some(entry) = self.reactions.get_mut(&key) {
            // Already in this state — no transition needed
            if entry.state == new_state {
                return false;
            }

            // Check debounce
            if let Some(last_call) = entry.last_api_call {
                if now.duration_since(last_call) < self.debounce {
                    return false; // Too soon, skip this transition
                }
            }

            entry.state = new_state;
            entry.state_changed_at = now;
            entry.last_api_call = Some(now);
            entry.api_call_count += 1;
            true
        } else {
            // New reaction
            self.reactions.insert(
                key,
                ReactionEntry {
                    state: new_state,
                    state_changed_at: now,
                    last_api_call: Some(now),
                    api_call_count: 1,
                },
            );
            true
        }
    }

    /// Get the current state of a reaction.
    pub fn state(&self, channel: &str, message_id: &str) -> Option<&ReactionState> {
        let key = (channel.to_string(), message_id.to_string());
        self.reactions.get(&key).map(|e| &e.state)
    }

    /// Prune stale reactions (older than watchdog timeout).
    /// Returns the number of pruned entries.
    pub fn prune_stale(&mut self) -> usize {
        let now = Instant::now();
        let before = self.reactions.len();

        self.reactions
            .retain(|_, entry| now.duration_since(entry.state_changed_at) < self.watchdog_timeout);

        before - self.reactions.len()
    }

    /// Get the number of tracked reactions.
    pub fn active_count(&self) -> usize {
        self.reactions.len()
    }

    /// Remove a specific reaction tracking entry.
    pub fn remove(&mut self, channel: &str, message_id: &str) {
        let key = (channel.to_string(), message_id.to_string());
        self.reactions.remove(&key);
    }
}

impl Default for ReactionStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_transition() {
        let mut sm = ReactionStateMachine::new();
        assert!(sm.transition("tg", "msg1", ReactionState::Acknowledged));
        assert_eq!(sm.state("tg", "msg1"), Some(&ReactionState::Acknowledged));
    }

    #[test]
    fn test_same_state_no_transition() {
        let mut sm = ReactionStateMachine::new();
        sm.transition("tg", "msg1", ReactionState::Acknowledged);
        assert!(!sm.transition("tg", "msg1", ReactionState::Acknowledged));
    }

    #[test]
    fn test_state_progression() {
        let mut sm = ReactionStateMachine::with_timing(0, 300); // No debounce for tests
        sm.transition("tg", "msg1", ReactionState::Acknowledged);
        assert!(sm.transition("tg", "msg1", ReactionState::Processing));
        assert!(sm.transition("tg", "msg1", ReactionState::Done));
        assert_eq!(sm.state("tg", "msg1"), Some(&ReactionState::Done));
    }

    #[test]
    fn test_active_count() {
        let mut sm = ReactionStateMachine::new();
        sm.transition("tg", "msg1", ReactionState::Acknowledged);
        sm.transition("tg", "msg2", ReactionState::Acknowledged);
        assert_eq!(sm.active_count(), 2);
    }

    #[test]
    fn test_remove_reaction() {
        let mut sm = ReactionStateMachine::new();
        sm.transition("tg", "msg1", ReactionState::Acknowledged);
        sm.remove("tg", "msg1");
        assert_eq!(sm.active_count(), 0);
    }

    #[test]
    fn test_prune_stale() {
        let mut sm = ReactionStateMachine::with_timing(0, 0); // 0s watchdog
        sm.transition("tg", "msg1", ReactionState::Acknowledged);
        // Sleep is not needed — 0s timeout means everything is stale immediately
        std::thread::sleep(Duration::from_millis(10));
        let pruned = sm.prune_stale();
        assert_eq!(pruned, 1);
        assert_eq!(sm.active_count(), 0);
    }

    #[test]
    fn test_debounce() {
        let mut sm = ReactionStateMachine::with_timing(1000, 300); // 1s debounce
        sm.transition("tg", "msg1", ReactionState::Acknowledged);
        // Second transition within debounce should be denied
        assert!(!sm.transition("tg", "msg1", ReactionState::Processing));
    }
}
