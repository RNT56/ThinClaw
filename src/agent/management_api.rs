//! Agent management API backend.
//!
//! Structured data layer for CRUD on agent summaries, default tracking,
//! and status updates. Backs CLI `agents` commands and Scrappy sidebar.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Agent status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentStatus {
    Idle,
    Busy { job_count: u32 },
    Paused,
    Offline,
}

impl AgentStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Idle => "idle",
            Self::Busy { .. } => "busy",
            Self::Paused => "paused",
            Self::Offline => "offline",
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Busy { .. })
    }
}

/// Summary of an agent for management purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_default: bool,
    pub status: AgentStatus,
    pub active_jobs: u32,
    pub channels: Vec<String>,
    pub created_at: String,
    pub model: Option<String>,
}

/// Agent management store.
pub struct AgentManagementStore {
    agents: HashMap<String, AgentSummary>,
    default_id: Option<String>,
}

impl AgentManagementStore {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            default_id: None,
        }
    }

    /// Register a new agent.
    pub fn register(&mut self, summary: AgentSummary) {
        let id = summary.id.clone();
        if summary.is_default {
            self.default_id = Some(id.clone());
        }
        self.agents.insert(id, summary);
    }

    /// Get agent by ID.
    pub fn get(&self, id: &str) -> Option<&AgentSummary> {
        self.agents.get(id)
    }

    /// List all agents.
    pub fn list(&self) -> Vec<&AgentSummary> {
        self.agents.values().collect()
    }

    /// Set the default agent. Returns false if ID not found.
    pub fn set_default(&mut self, id: &str) -> bool {
        if !self.agents.contains_key(id) {
            return false;
        }
        // Clear old default
        if let Some(old_id) = &self.default_id {
            if let Some(old) = self.agents.get_mut(old_id) {
                old.is_default = false;
            }
        }
        self.default_id = Some(id.to_string());
        if let Some(agent) = self.agents.get_mut(id) {
            agent.is_default = true;
        }
        true
    }

    /// Get the default agent.
    pub fn get_default(&self) -> Option<&AgentSummary> {
        self.default_id.as_ref().and_then(|id| self.agents.get(id))
    }

    /// Update agent status. Returns false if ID not found.
    pub fn update_status(&mut self, id: &str, status: AgentStatus) -> bool {
        if let Some(agent) = self.agents.get_mut(id) {
            agent.status = status;
            true
        } else {
            false
        }
    }

    /// Remove agent by ID.
    pub fn remove(&mut self, id: &str) -> Option<AgentSummary> {
        let removed = self.agents.remove(id);
        if self.default_id.as_deref() == Some(id) {
            self.default_id = None;
        }
        removed
    }

    /// Get agents that are not idle.
    pub fn active_agents(&self) -> Vec<&AgentSummary> {
        self.agents
            .values()
            .filter(|a| a.status.is_active())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.agents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }
}

impl Default for AgentManagementStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_agent(id: &str) -> AgentSummary {
        AgentSummary {
            id: id.into(),
            name: format!("Agent {}", id),
            description: None,
            is_default: false,
            status: AgentStatus::Idle,
            active_jobs: 0,
            channels: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
            model: None,
        }
    }

    #[test]
    fn test_register_and_get() {
        let mut store = AgentManagementStore::new();
        store.register(make_agent("a1"));
        assert!(store.get("a1").is_some());
    }

    #[test]
    fn test_list_all() {
        let mut store = AgentManagementStore::new();
        store.register(make_agent("a1"));
        store.register(make_agent("a2"));
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn test_set_default() {
        let mut store = AgentManagementStore::new();
        store.register(make_agent("a1"));
        store.register(make_agent("a2"));
        assert!(store.set_default("a1"));
        assert!(store.get("a1").unwrap().is_default);
    }

    #[test]
    fn test_get_default() {
        let mut store = AgentManagementStore::new();
        let mut agent = make_agent("a1");
        agent.is_default = true;
        store.register(agent);
        assert_eq!(store.get_default().unwrap().id, "a1");
    }

    #[test]
    fn test_update_status() {
        let mut store = AgentManagementStore::new();
        store.register(make_agent("a1"));
        assert!(store.update_status("a1", AgentStatus::Busy { job_count: 2 }));
        assert!(store.get("a1").unwrap().status.is_active());
    }

    #[test]
    fn test_remove() {
        let mut store = AgentManagementStore::new();
        store.register(make_agent("a1"));
        assert!(store.remove("a1").is_some());
        assert!(store.is_empty());
    }

    #[test]
    fn test_active_agents() {
        let mut store = AgentManagementStore::new();
        store.register(make_agent("idle"));
        let mut busy = make_agent("busy");
        busy.status = AgentStatus::Busy { job_count: 1 };
        store.register(busy);
        assert_eq!(store.active_agents().len(), 1);
    }

    #[test]
    fn test_set_default_nonexistent() {
        let mut store = AgentManagementStore::new();
        assert!(!store.set_default("none"));
    }

    #[test]
    fn test_agent_status_variants() {
        assert_eq!(AgentStatus::Idle.label(), "idle");
        assert_eq!(AgentStatus::Busy { job_count: 1 }.label(), "busy");
        assert_eq!(AgentStatus::Paused.label(), "paused");
        assert_eq!(AgentStatus::Offline.label(), "offline");
    }
}
