//! Agent presence system.
//!
//! Tracks agent and device online status via periodic beacons.
//! Supports system-level presence for multi-agent coordination.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Presence status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PresenceStatus {
    Online,
    Away,
    Busy,
    Offline,
}

/// A presence beacon from an agent or device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceBeacon {
    /// Entity ID (agent or device).
    pub entity_id: String,
    /// Entity type.
    pub entity_type: EntityType,
    /// Current status.
    pub status: PresenceStatus,
    /// Custom status message.
    pub status_message: Option<String>,
    /// Timestamp of the beacon (RFC 3339).
    pub timestamp: String,
    /// Capabilities this entity offers.
    pub capabilities: Vec<String>,
    /// Metadata.
    pub metadata: HashMap<String, String>,
}

/// Type of entity sending presence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EntityType {
    Agent,
    Device,
    System,
}

/// Presence tracker.
pub struct PresenceTracker {
    entries: HashMap<String, PresenceBeacon>,
    /// Timeout after which an entity is considered offline (seconds).
    timeout_secs: u64,
}

impl PresenceTracker {
    pub fn new(timeout_secs: u64) -> Self {
        Self {
            entries: HashMap::new(),
            timeout_secs,
        }
    }

    /// Update presence for an entity.
    pub fn update(&mut self, beacon: PresenceBeacon) {
        self.entries.insert(beacon.entity_id.clone(), beacon);
    }

    /// Get presence for an entity.
    pub fn get(&self, entity_id: &str) -> Option<&PresenceBeacon> {
        self.entries.get(entity_id)
    }

    /// Set an entity offline.
    pub fn set_offline(&mut self, entity_id: &str) {
        if let Some(entry) = self.entries.get_mut(entity_id) {
            entry.status = PresenceStatus::Offline;
        }
    }

    /// List all entities by status.
    pub fn by_status(&self, status: &PresenceStatus) -> Vec<&PresenceBeacon> {
        self.entries
            .values()
            .filter(|b| &b.status == status)
            .collect()
    }

    /// List all online entities.
    pub fn online(&self) -> Vec<&PresenceBeacon> {
        self.entries
            .values()
            .filter(|b| b.status != PresenceStatus::Offline)
            .collect()
    }

    /// List online agents.
    pub fn online_agents(&self) -> Vec<&PresenceBeacon> {
        self.online()
            .into_iter()
            .filter(|b| b.entity_type == EntityType::Agent)
            .collect()
    }

    /// Total entries.
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// Timeout value.
    pub fn timeout_secs(&self) -> u64 {
        self.timeout_secs
    }

    /// Prune stale entries based on a reference timestamp.
    pub fn prune_stale(&mut self, now_epoch_secs: u64) -> usize {
        let before = self.entries.len();
        self.entries.retain(|_, beacon| {
            // Parse timestamp as epoch seconds for comparison
            if let Ok(ts) = beacon.timestamp.parse::<u64>() {
                now_epoch_secs.saturating_sub(ts) < self.timeout_secs
            } else {
                true // Keep entries with non-numeric timestamps
            }
        });
        before - self.entries.len()
    }
}

impl Default for PresenceTracker {
    fn default() -> Self {
        Self::new(300) // 5 minute timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_beacon(id: &str, status: PresenceStatus) -> PresenceBeacon {
        PresenceBeacon {
            entity_id: id.to_string(),
            entity_type: EntityType::Agent,
            status,
            status_message: None,
            timestamp: "1000".to_string(),
            capabilities: vec!["chat".to_string()],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn test_update_and_get() {
        let mut tracker = PresenceTracker::default();
        tracker.update(test_beacon("agent-1", PresenceStatus::Online));
        assert!(tracker.get("agent-1").is_some());
    }

    #[test]
    fn test_set_offline() {
        let mut tracker = PresenceTracker::default();
        tracker.update(test_beacon("agent-1", PresenceStatus::Online));
        tracker.set_offline("agent-1");
        assert_eq!(
            tracker.get("agent-1").unwrap().status,
            PresenceStatus::Offline
        );
    }

    #[test]
    fn test_online_count() {
        let mut tracker = PresenceTracker::default();
        tracker.update(test_beacon("a", PresenceStatus::Online));
        tracker.update(test_beacon("b", PresenceStatus::Offline));
        tracker.update(test_beacon("c", PresenceStatus::Busy));
        assert_eq!(tracker.online().len(), 2);
    }

    #[test]
    fn test_online_agents() {
        let mut tracker = PresenceTracker::default();
        tracker.update(test_beacon("a", PresenceStatus::Online));

        let mut device_beacon = test_beacon("d", PresenceStatus::Online);
        device_beacon.entity_type = EntityType::Device;
        tracker.update(device_beacon);

        assert_eq!(tracker.online_agents().len(), 1);
    }

    #[test]
    fn test_prune_stale() {
        let mut tracker = PresenceTracker::new(60);
        tracker.update(test_beacon("old", PresenceStatus::Online));
        let pruned = tracker.prune_stale(2000);
        assert_eq!(pruned, 1);
    }

    #[test]
    fn test_by_status() {
        let mut tracker = PresenceTracker::default();
        tracker.update(test_beacon("a", PresenceStatus::Online));
        tracker.update(test_beacon("b", PresenceStatus::Busy));
        assert_eq!(tracker.by_status(&PresenceStatus::Online).len(), 1);
    }
}
