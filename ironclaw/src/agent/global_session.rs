//! Global sessions — optional shared context across channels.
//!
//! When enabled, a "global session" stores context that persists across
//! all channels and threads. This is useful for agents that need to
//! maintain cross-channel state (e.g., a project status tracker).
//!
//! Configuration:
//! - `GLOBAL_SESSION_ENABLED` — enable global session (default: false)
//! - `GLOBAL_SESSION_MAX_ENTRIES` — max entries in global context (default: 100)

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Configuration for global sessions.
#[derive(Debug, Clone)]
pub struct GlobalSessionConfig {
    pub enabled: bool,
    pub max_entries: usize,
}

impl Default for GlobalSessionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries: 100,
        }
    }
}

impl GlobalSessionConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(val) = std::env::var("GLOBAL_SESSION_ENABLED") {
            config.enabled = val == "1" || val.eq_ignore_ascii_case("true");
        }
        if let Ok(max) = std::env::var("GLOBAL_SESSION_MAX_ENTRIES") {
            if let Ok(m) = max.parse() {
                config.max_entries = m;
            }
        }
        config
    }
}

/// An entry in the global session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalEntry {
    /// Unique key for this entry.
    pub key: String,
    /// The stored value.
    pub value: String,
    /// Source channel that created this entry.
    pub source_channel: String,
    /// When the entry was created.
    pub created_at: DateTime<Utc>,
    /// When the entry was last updated.
    pub updated_at: DateTime<Utc>,
    /// Tags for categorization.
    pub tags: Vec<String>,
}

/// Global session store.
pub struct GlobalSession {
    config: GlobalSessionConfig,
    entries: VecDeque<GlobalEntry>,
}

impl GlobalSession {
    pub fn new(config: GlobalSessionConfig) -> Self {
        Self {
            config,
            entries: VecDeque::new(),
        }
    }

    /// Store or update a global entry.
    pub fn upsert(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        channel: impl Into<String>,
    ) {
        if !self.config.enabled {
            return;
        }

        let key = key.into();
        let now = Utc::now();

        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.value = value.into();
            entry.updated_at = now;
        } else {
            if self.entries.len() >= self.config.max_entries {
                self.entries.pop_front(); // Remove oldest
            }
            self.entries.push_back(GlobalEntry {
                key,
                value: value.into(),
                source_channel: channel.into(),
                created_at: now,
                updated_at: now,
                tags: Vec::new(),
            });
        }
    }

    /// Get an entry by key.
    pub fn get(&self, key: &str) -> Option<&GlobalEntry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// Remove an entry by key.
    pub fn remove(&mut self, key: &str) -> bool {
        if let Some(pos) = self.entries.iter().position(|e| e.key == key) {
            self.entries.remove(pos);
            true
        } else {
            false
        }
    }

    /// Get all entries, optionally filtered by tag.
    pub fn list(&self, tag_filter: Option<&str>) -> Vec<&GlobalEntry> {
        self.entries
            .iter()
            .filter(|e| tag_filter.map_or(true, |tag| e.tags.iter().any(|t| t == tag)))
            .collect()
    }

    /// Build a context string for injection into session context.
    pub fn to_context_string(&self) -> String {
        if !self.config.enabled || self.entries.is_empty() {
            return String::new();
        }

        let mut lines = vec!["[Global Context]".to_string()];
        for entry in &self.entries {
            lines.push(format!("  {}: {}", entry.key, entry.value));
        }
        lines.join("\n")
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear all entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> GlobalSessionConfig {
        GlobalSessionConfig {
            enabled: true,
            max_entries: 5,
        }
    }

    #[test]
    fn test_upsert_and_get() {
        let mut session = GlobalSession::new(test_config());
        session.upsert("project", "IronClaw", "telegram");

        let entry = session.get("project").unwrap();
        assert_eq!(entry.value, "IronClaw");
        assert_eq!(entry.source_channel, "telegram");
    }

    #[test]
    fn test_upsert_updates() {
        let mut session = GlobalSession::new(test_config());
        session.upsert("status", "building", "tg");
        session.upsert("status", "deployed", "discord");

        let entry = session.get("status").unwrap();
        assert_eq!(entry.value, "deployed");
        assert_eq!(session.len(), 1); // No duplicate
    }

    #[test]
    fn test_max_entries_eviction() {
        let mut session = GlobalSession::new(test_config());
        for i in 0..10 {
            session.upsert(format!("key-{}", i), format!("val-{}", i), "tg");
        }
        assert_eq!(session.len(), 5); // Capped at max
        assert!(session.get("key-0").is_none()); // Oldest evicted
        assert!(session.get("key-9").is_some()); // Newest kept
    }

    #[test]
    fn test_remove() {
        let mut session = GlobalSession::new(test_config());
        session.upsert("x", "y", "tg");
        assert!(session.remove("x"));
        assert!(!session.remove("x")); // Already gone
    }

    #[test]
    fn test_disabled_no_ops() {
        let config = GlobalSessionConfig::default(); // disabled
        let mut session = GlobalSession::new(config);
        session.upsert("key", "val", "tg");
        assert!(session.is_empty());
    }

    #[test]
    fn test_context_string() {
        let mut session = GlobalSession::new(test_config());
        session.upsert("project", "IronClaw", "tg");
        session.upsert("sprint", "9", "discord");

        let ctx = session.to_context_string();
        assert!(ctx.contains("[Global Context]"));
        assert!(ctx.contains("project: IronClaw"));
        assert!(ctx.contains("sprint: 9"));
    }

    #[test]
    fn test_empty_context_string() {
        let session = GlobalSession::new(test_config());
        assert!(session.to_context_string().is_empty());
    }
}
