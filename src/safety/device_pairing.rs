//! Device pairing — trust establishment for multi-device setups.
//!
//! Before a new device can connect, it must be paired via a
//! challenge-response handshake.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Pairing states.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PairingState {
    /// Pairing request sent, awaiting approval.
    Pending,
    /// Paired and trusted.
    Paired,
    /// Pairing was rejected.
    Rejected,
    /// Pairing was revoked.
    Revoked,
}

/// A pairing record for a device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRecord {
    /// Device ID.
    pub device_id: String,
    /// Device name.
    pub device_name: String,
    /// Public key fingerprint.
    pub fingerprint: String,
    /// Current pairing state.
    pub state: PairingState,
    /// When pairing was initiated.
    pub created_at: String,
    /// When pairing was last updated.
    pub updated_at: String,
    /// Platform (macos, linux, ios, android).
    pub platform: Option<String>,
}

/// Device pairing store.
pub struct PairingStore {
    records: HashMap<String, PairingRecord>,
}

impl PairingStore {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    /// Initiate a pairing request.
    pub fn request_pairing(&mut self, record: PairingRecord) {
        self.records.insert(record.device_id.clone(), record);
    }

    /// Approve a pending pairing.
    pub fn approve(&mut self, device_id: &str, timestamp: &str) -> bool {
        if let Some(record) = self.records.get_mut(device_id) {
            if record.state == PairingState::Pending {
                record.state = PairingState::Paired;
                record.updated_at = timestamp.to_string();
                return true;
            }
        }
        false
    }

    /// Reject a pending pairing.
    pub fn reject(&mut self, device_id: &str, timestamp: &str) -> bool {
        if let Some(record) = self.records.get_mut(device_id) {
            if record.state == PairingState::Pending {
                record.state = PairingState::Rejected;
                record.updated_at = timestamp.to_string();
                return true;
            }
        }
        false
    }

    /// Revoke an existing pairing.
    pub fn revoke(&mut self, device_id: &str, timestamp: &str) -> bool {
        if let Some(record) = self.records.get_mut(device_id) {
            if record.state == PairingState::Paired {
                record.state = PairingState::Revoked;
                record.updated_at = timestamp.to_string();
                return true;
            }
        }
        false
    }

    /// Check if a device is paired.
    pub fn is_paired(&self, device_id: &str) -> bool {
        self.records
            .get(device_id)
            .map(|r| r.state == PairingState::Paired)
            .unwrap_or(false)
    }

    /// Verify a device by fingerprint.
    pub fn verify_fingerprint(&self, device_id: &str, fingerprint: &str) -> bool {
        self.records
            .get(device_id)
            .map(|r| r.state == PairingState::Paired && r.fingerprint == fingerprint)
            .unwrap_or(false)
    }

    /// List pending pairing requests.
    pub fn pending(&self) -> Vec<&PairingRecord> {
        self.records
            .values()
            .filter(|r| r.state == PairingState::Pending)
            .collect()
    }

    /// List paired devices.
    pub fn paired(&self) -> Vec<&PairingRecord> {
        self.records
            .values()
            .filter(|r| r.state == PairingState::Paired)
            .collect()
    }

    /// Get a record.
    pub fn get(&self, device_id: &str) -> Option<&PairingRecord> {
        self.records.get(device_id)
    }

    /// Remove a record entirely.
    pub fn remove(&mut self, device_id: &str) -> Option<PairingRecord> {
        self.records.remove(device_id)
    }

    /// Count all records.
    pub fn count(&self) -> usize {
        self.records.len()
    }
}

impl Default for PairingStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a pairing code (6-digit numeric).
pub fn generate_pairing_code() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{:06}", nanos % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_record(id: &str) -> PairingRecord {
        PairingRecord {
            device_id: id.to_string(),
            device_name: "Test Device".to_string(),
            fingerprint: "SHA256:abc123".to_string(),
            state: PairingState::Pending,
            created_at: "2026-01-01".to_string(),
            updated_at: "2026-01-01".to_string(),
            platform: Some("macos".to_string()),
        }
    }

    #[test]
    fn test_request_and_approve() {
        let mut store = PairingStore::new();
        store.request_pairing(test_record("d1"));
        assert!(!store.is_paired("d1"));
        assert!(store.approve("d1", "now"));
        assert!(store.is_paired("d1"));
    }

    #[test]
    fn test_reject() {
        let mut store = PairingStore::new();
        store.request_pairing(test_record("d1"));
        assert!(store.reject("d1", "now"));
        assert!(!store.is_paired("d1"));
    }

    #[test]
    fn test_revoke() {
        let mut store = PairingStore::new();
        store.request_pairing(test_record("d1"));
        store.approve("d1", "now");
        assert!(store.revoke("d1", "now"));
        assert!(!store.is_paired("d1"));
    }

    #[test]
    fn test_verify_fingerprint() {
        let mut store = PairingStore::new();
        store.request_pairing(test_record("d1"));
        store.approve("d1", "now");
        assert!(store.verify_fingerprint("d1", "SHA256:abc123"));
        assert!(!store.verify_fingerprint("d1", "wrong"));
    }

    #[test]
    fn test_pending_list() {
        let mut store = PairingStore::new();
        store.request_pairing(test_record("d1"));
        store.request_pairing(test_record("d2"));
        store.approve("d1", "now");
        assert_eq!(store.pending().len(), 1);
        assert_eq!(store.paired().len(), 1);
    }

    #[test]
    fn test_generate_pairing_code() {
        let code = generate_pairing_code();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }
}
