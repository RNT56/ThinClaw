//! Thread parent binding inheritance.
//!
//! In threaded conversations, child threads inherit routing rules from
//! their parent. This prevents agents from losing context when users
//! create sub-threads.

use std::collections::HashMap;

/// Thread binding — maps a child thread to its parent.
#[derive(Debug, Clone)]
pub struct ThreadBinding {
    /// ID of the child thread.
    pub child_thread_id: String,
    /// ID of the parent thread.
    pub parent_thread_id: String,
    /// Channel the thread belongs to.
    pub channel: String,
    /// Agent ID that owns the parent thread (if any).
    pub inherited_agent_id: Option<String>,
}

/// Thread inheritance tracker.
pub struct ThreadInheritance {
    /// Map from child thread ID → parent thread ID.
    bindings: HashMap<String, ThreadBinding>,
}

impl ThreadInheritance {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Register a parent-child thread relationship.
    pub fn bind(
        &mut self,
        child: impl Into<String>,
        parent: impl Into<String>,
        channel: impl Into<String>,
        agent_id: Option<String>,
    ) {
        let child = child.into();
        self.bindings.insert(
            child.clone(),
            ThreadBinding {
                child_thread_id: child,
                parent_thread_id: parent.into(),
                channel: channel.into(),
                inherited_agent_id: agent_id,
            },
        );
    }

    /// Resolve the effective parent for a thread (follows chain up).
    pub fn resolve_parent<'a>(&'a self, thread_id: &'a str) -> Option<&'a str> {
        let mut current = thread_id;
        let mut depth = 0;
        const MAX_DEPTH: usize = 10;

        while let Some(binding) = self.bindings.get(current) {
            current = &binding.parent_thread_id;
            depth += 1;
            if depth >= MAX_DEPTH {
                break; // Prevent infinite loops
            }
        }

        if current != thread_id {
            Some(current)
        } else {
            None
        }
    }

    /// Get the inherited agent ID for a thread.
    pub fn inherited_agent(&self, thread_id: &str) -> Option<&str> {
        // Walk up the chain looking for an agent assignment
        let mut current = thread_id;
        let mut depth = 0;
        const MAX_DEPTH: usize = 10;

        while let Some(binding) = self.bindings.get(current) {
            if let Some(ref agent) = binding.inherited_agent_id {
                return Some(agent);
            }
            current = &binding.parent_thread_id;
            depth += 1;
            if depth >= MAX_DEPTH {
                break;
            }
        }

        None
    }

    /// Remove a binding.
    pub fn unbind(&mut self, child_thread_id: &str) {
        self.bindings.remove(child_thread_id);
    }

    /// Get the number of tracked bindings.
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// Prune bindings for a specific channel.
    pub fn prune_channel(&mut self, channel: &str) -> usize {
        let before = self.bindings.len();
        self.bindings.retain(|_, b| b.channel != channel);
        before - self.bindings.len()
    }
}

impl Default for ThreadInheritance {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bind_and_resolve() {
        let mut tracker = ThreadInheritance::new();
        tracker.bind("child-1", "parent-1", "telegram", None);

        assert_eq!(tracker.resolve_parent("child-1"), Some("parent-1"));
        assert_eq!(tracker.resolve_parent("unknown"), None);
    }

    #[test]
    fn test_chain_resolution() {
        let mut tracker = ThreadInheritance::new();
        tracker.bind("grandchild", "child", "tg", None);
        tracker.bind("child", "parent", "tg", None);

        // grandchild → child → parent
        assert_eq!(tracker.resolve_parent("grandchild"), Some("parent"));
    }

    #[test]
    fn test_inherited_agent() {
        let mut tracker = ThreadInheritance::new();
        tracker.bind("child", "parent", "tg", Some("agent-1".to_string()));

        assert_eq!(tracker.inherited_agent("child"), Some("agent-1"));
        assert_eq!(tracker.inherited_agent("unknown"), None);
    }

    #[test]
    fn test_agent_inheritance_chain() {
        let mut tracker = ThreadInheritance::new();
        tracker.bind("grandchild", "child", "tg", None);
        tracker.bind("child", "parent", "tg", Some("agent-x".to_string()));

        // grandchild inherits from child's binding
        assert_eq!(tracker.inherited_agent("grandchild"), Some("agent-x"));
    }

    #[test]
    fn test_unbind() {
        let mut tracker = ThreadInheritance::new();
        tracker.bind("child", "parent", "tg", None);
        tracker.unbind("child");
        assert_eq!(tracker.binding_count(), 0);
    }

    #[test]
    fn test_prune_channel() {
        let mut tracker = ThreadInheritance::new();
        tracker.bind("t1", "p1", "telegram", None);
        tracker.bind("t2", "p2", "telegram", None);
        tracker.bind("t3", "p3", "discord", None);

        let pruned = tracker.prune_channel("telegram");
        assert_eq!(pruned, 2);
        assert_eq!(tracker.binding_count(), 1);
    }

    #[test]
    fn test_max_depth_protection() {
        let mut tracker = ThreadInheritance::new();
        // Create a cycle (shouldn't happen, but be safe)
        tracker.bind("a", "b", "tg", None);
        tracker.bind("b", "a", "tg", None);

        // Should not infinite loop
        let _ = tracker.resolve_parent("a");
    }
}
