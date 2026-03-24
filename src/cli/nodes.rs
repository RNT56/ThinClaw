//! `nodes` CLI — device management.
//!
//! Provides `nodes list`, `nodes show`, `nodes remove`, `nodes clear`
//! subcommands for managing connected devices/nodes.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A connected node/device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// Node ID (unique identifier).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Platform (e.g., "macos", "linux", "ios", "android").
    pub platform: String,
    /// Whether this node is currently online.
    pub online: bool,
    /// Last seen timestamp (RFC 3339).
    pub last_seen: String,
    /// IP address (if available).
    pub ip_address: Option<String>,
    /// Node version.
    pub version: Option<String>,
    /// Node capabilities.
    pub capabilities: Vec<String>,
}

/// Node management store.
pub struct NodeStore {
    nodes: HashMap<String, Node>,
}

impl NodeStore {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Register or update a node.
    pub fn upsert(&mut self, node: Node) {
        self.nodes.insert(node.id.clone(), node);
    }

    /// Get a node by ID.
    pub fn get(&self, id: &str) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Remove a node.
    pub fn remove(&mut self, id: &str) -> Option<Node> {
        self.nodes.remove(id)
    }

    /// Clear all nodes.
    pub fn clear(&mut self) -> usize {
        let count = self.nodes.len();
        self.nodes.clear();
        count
    }

    /// List all nodes.
    pub fn list(&self) -> Vec<&Node> {
        let mut nodes: Vec<_> = self.nodes.values().collect();
        nodes.sort_by(|a, b| a.name.cmp(&b.name));
        nodes
    }

    /// List online nodes.
    pub fn online(&self) -> Vec<&Node> {
        self.list().into_iter().filter(|n| n.online).collect()
    }

    /// List offline nodes.
    pub fn offline(&self) -> Vec<&Node> {
        self.list().into_iter().filter(|n| !n.online).collect()
    }

    /// Total node count.
    pub fn count(&self) -> usize {
        self.nodes.len()
    }

    /// Mark a node as offline.
    pub fn mark_offline(&mut self, id: &str) -> bool {
        if let Some(node) = self.nodes.get_mut(id) {
            node.online = false;
            true
        } else {
            false
        }
    }

    /// Mark a node as online.
    pub fn mark_online(&mut self, id: &str, last_seen: &str) -> bool {
        if let Some(node) = self.nodes.get_mut(id) {
            node.online = true;
            node.last_seen = last_seen.to_string();
            true
        } else {
            false
        }
    }
}

impl Default for NodeStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a node list for display.
pub fn format_node_list(nodes: &[&Node]) -> String {
    if nodes.is_empty() {
        return "No nodes registered.".to_string();
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "{:<20} {:<10} {:<8} {:<20}",
        "NAME", "PLATFORM", "STATUS", "LAST SEEN"
    ));
    lines.push("-".repeat(60));

    for node in nodes {
        let status = if node.online { "online" } else { "offline" };
        lines.push(format!(
            "{:<20} {:<10} {:<8} {:<20}",
            node.name, node.platform, status, node.last_seen
        ));
    }

    lines.join("\n")
}

/// Format a single node for display.
pub fn format_node_detail(node: &Node) -> String {
    let mut lines = vec![
        format!("Node: {}", node.name),
        format!("  ID:       {}", node.id),
        format!("  Platform: {}", node.platform),
        format!(
            "  Status:   {}",
            if node.online { "online" } else { "offline" }
        ),
        format!("  Last Seen: {}", node.last_seen),
    ];

    if let Some(ip) = &node.ip_address {
        lines.push(format!("  IP:       {}", ip));
    }
    if let Some(ver) = &node.version {
        lines.push(format!("  Version:  {}", ver));
    }
    if !node.capabilities.is_empty() {
        lines.push(format!("  Caps:     {}", node.capabilities.join(", ")));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_node(id: &str, name: &str, online: bool) -> Node {
        Node {
            id: id.to_string(),
            name: name.to_string(),
            platform: "macos".to_string(),
            online,
            last_seen: "2026-03-04T08:00:00Z".to_string(),
            ip_address: Some("192.168.1.1".to_string()),
            version: Some("1.0.0".to_string()),
            capabilities: vec!["tts".to_string(), "shell".to_string()],
        }
    }

    #[test]
    fn test_upsert_and_get() {
        let mut store = NodeStore::new();
        store.upsert(test_node("n1", "MacBook", true));
        assert!(store.get("n1").is_some());
        assert_eq!(store.get("n1").unwrap().name, "MacBook");
    }

    #[test]
    fn test_remove() {
        let mut store = NodeStore::new();
        store.upsert(test_node("n1", "MacBook", true));
        assert!(store.remove("n1").is_some());
        assert!(store.get("n1").is_none());
    }

    #[test]
    fn test_clear() {
        let mut store = NodeStore::new();
        store.upsert(test_node("n1", "MacBook", true));
        store.upsert(test_node("n2", "iPhone", true));
        let cleared = store.clear();
        assert_eq!(cleared, 2);
        assert_eq!(store.count(), 0);
    }

    #[test]
    fn test_online_offline() {
        let mut store = NodeStore::new();
        store.upsert(test_node("n1", "MacBook", true));
        store.upsert(test_node("n2", "iPhone", false));

        assert_eq!(store.online().len(), 1);
        assert_eq!(store.offline().len(), 1);
    }

    #[test]
    fn test_mark_offline() {
        let mut store = NodeStore::new();
        store.upsert(test_node("n1", "MacBook", true));
        store.mark_offline("n1");
        assert!(!store.get("n1").unwrap().online);
    }

    #[test]
    fn test_mark_online() {
        let mut store = NodeStore::new();
        store.upsert(test_node("n1", "MacBook", false));
        store.mark_online("n1", "2026-03-04T09:00:00Z");
        let node = store.get("n1").unwrap();
        assert!(node.online);
        assert_eq!(node.last_seen, "2026-03-04T09:00:00Z");
    }

    #[test]
    fn test_format_empty_list() {
        let formatted = format_node_list(&[]);
        assert!(formatted.contains("No nodes"));
    }

    #[test]
    fn test_format_node_list() {
        let node = test_node("n1", "MacBook", true);
        let formatted = format_node_list(&[&node]);
        assert!(formatted.contains("MacBook"));
        assert!(formatted.contains("online"));
    }

    #[test]
    fn test_format_node_detail() {
        let node = test_node("n1", "MacBook", true);
        let detail = format_node_detail(&node);
        assert!(detail.contains("MacBook"));
        assert!(detail.contains("192.168.1.1"));
        assert!(detail.contains("tts"));
    }

    #[test]
    fn test_count() {
        let mut store = NodeStore::new();
        assert_eq!(store.count(), 0);
        store.upsert(test_node("n1", "MacBook", true));
        assert_eq!(store.count(), 1);
    }
}
