//! In-memory device index over [`DeviceStore`], used on the request hot
//! path so authentication never does file I/O per request.
//!
//! - `authenticate` hashes the presented bearer token, looks it up by hash
//!   in a `HashMap`, then does a constant-time compare of the stored hash
//!   bytes against the freshly computed hash bytes (belt-and-suspenders:
//!   the `HashMap` lookup already requires an exact hash match, but D-T2
//!   asks for `ct_eq` on the final compare, mirroring
//!   `crates/thinclaw-gateway/src/web/auth.rs`).
//! - `last_seen` updates are applied in memory immediately and flushed to
//!   disk at most once per 60s per device, so a chatty client doesn't
//!   generate a disk write (and file-lock contention) per request.
//! - `revoke` also broadcasts the revoked `device_id` on a `tokio::sync::
//!   broadcast` channel so SSE/WS stream handlers can subscribe and tear
//!   down live connections synchronously (gateway hardening item 5).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use subtle::ConstantTimeEq;
use tokio::sync::{RwLock, broadcast};

use super::store::{DeviceStore, DeviceStoreError, hash_token};
use super::types::{DeviceRecord, DeviceScope};

/// Debounce window for flushing `last_seen_at` updates to disk.
const LAST_SEEN_FLUSH_INTERVAL: Duration = Duration::from_secs(60);

/// Capacity of the revocation broadcast channel. Generous relative to the
/// expected number of concurrently-streaming device connections; lagging
/// subscribers just miss older revocations and must re-check on next use.
const REVOCATION_CHANNEL_CAPACITY: usize = 64;

/// Successful authentication result: the device's identity and granted
/// scopes, ready for scope-middleware checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuth {
    pub device_id: String,
    pub scopes: Vec<DeviceScope>,
}

#[derive(Clone)]
struct IndexEntry {
    device_id: String,
    token_hash: String,
    scopes: Vec<DeviceScope>,
    blocked: bool,
    expires_at: Option<String>,
}

struct LastSeenState {
    /// In-memory last-seen timestamp, updated on every authenticated
    /// request.
    pending: HashMap<String, String>,
    /// When each device's pending timestamp was last flushed to disk.
    last_flush: HashMap<String, Instant>,
}

/// In-memory, `Arc`-shareable index over the on-disk [`DeviceStore`].
#[derive(Clone)]
pub struct DeviceRegistry {
    store: Arc<DeviceStore>,
    by_hash: Arc<RwLock<HashMap<String, IndexEntry>>>,
    last_seen: Arc<RwLock<LastSeenState>>,
    revocations: broadcast::Sender<String>,
}

impl DeviceRegistry {
    /// Build a registry backed by `store`, loading its current contents
    /// into memory immediately.
    pub async fn load(store: DeviceStore) -> Result<Self, DeviceStoreError> {
        let store = Arc::new(store);
        let records = store.list()?;
        let mut by_hash = HashMap::with_capacity(records.len());
        for record in &records {
            by_hash.insert(record.token_hash.clone(), index_entry(record));
        }

        let (tx, _rx) = broadcast::channel(REVOCATION_CHANNEL_CAPACITY);

        Ok(Self {
            store,
            by_hash: Arc::new(RwLock::new(by_hash)),
            last_seen: Arc::new(RwLock::new(LastSeenState {
                pending: HashMap::new(),
                last_flush: HashMap::new(),
            })),
            revocations: tx,
        })
    }

    /// Re-read a single device from the store and refresh its index entry.
    /// Callers should invoke this after any store mutation that changes a
    /// device's token/scopes/revocation state (insert/rotate/revoke).
    pub async fn refresh(&self, device_id: &str) -> Result<(), DeviceStoreError> {
        let mut by_hash = self.by_hash.write().await;
        by_hash.retain(|_, entry| entry.device_id != device_id);
        if let Some(record) = self.store.get(device_id)? {
            by_hash.insert(record.token_hash.clone(), index_entry(&record));
        }
        Ok(())
    }

    /// Authenticate a raw bearer token. Hashes it, looks the hash up in the
    /// index, then does a constant-time compare against the stored hash
    /// before honoring the match. Rejects revoked or expired devices.
    pub async fn authenticate(&self, token: &str) -> Option<DeviceAuth> {
        let presented_hash = hash_token(token);
        let by_hash = self.by_hash.read().await;
        let entry = by_hash.get(&presented_hash)?;

        // Constant-time final compare (D-T2), even though the HashMap
        // lookup above already required an exact key match — this mirrors
        // the ct_eq discipline used at crates/thinclaw-gateway/src/web/auth.rs
        // and crates/orchestrator/auth.rs so the pattern stays consistent
        // if the lookup strategy ever changes.
        if !bool::from(entry.token_hash.as_bytes().ct_eq(presented_hash.as_bytes())) {
            return None;
        }

        if entry.blocked {
            return None;
        }

        if let Some(expires_at) = &entry.expires_at {
            let now = super::store::now_iso();
            if expires_at.as_str() <= now.as_str() {
                return None;
            }
        }

        Some(DeviceAuth {
            device_id: entry.device_id.clone(),
            scopes: entry.scopes.clone(),
        })
    }

    /// Record device activity. Updates the in-memory last-seen timestamp
    /// immediately; flushes to disk at most once per
    /// [`LAST_SEEN_FLUSH_INTERVAL`] per device.
    pub async fn touch(&self, device_id: &str, at: &str) -> Result<(), DeviceStoreError> {
        let mut state = self.last_seen.write().await;
        state.pending.insert(device_id.to_string(), at.to_string());

        let should_flush = match state.last_flush.get(device_id) {
            Some(last) => last.elapsed() >= LAST_SEEN_FLUSH_INTERVAL,
            None => true,
        };

        if should_flush {
            self.store.touch_last_seen(device_id, at)?;
            state
                .last_flush
                .insert(device_id.to_string(), Instant::now());
        }

        Ok(())
    }

    /// Force-flush any pending last-seen updates to disk (e.g. on clean
    /// shutdown).
    pub async fn flush_last_seen(&self) -> Result<(), DeviceStoreError> {
        let mut state = self.last_seen.write().await;
        for (device_id, at) in state.pending.iter() {
            self.store.touch_last_seen(device_id, at)?;
        }
        let now = Instant::now();
        let device_ids: Vec<String> = state.pending.keys().cloned().collect();
        for device_id in device_ids {
            state.last_flush.insert(device_id, now);
        }
        Ok(())
    }

    /// Revoke a device: persists the revocation, updates the in-memory
    /// index, and broadcasts the id so live stream handlers can disconnect
    /// it synchronously.
    pub async fn revoke(&self, device_id: &str) -> Result<DeviceRecord, DeviceStoreError> {
        let record = self.store.revoke(device_id)?;
        self.refresh(device_id).await?;
        // Best-effort: no receivers is not an error (nothing is streaming).
        let _ = self.revocations.send(device_id.to_string());
        Ok(record)
    }

    /// Subscribe to device-revocation notifications. Stream handlers
    /// (SSE/WS) should hold one receiver per active connection and
    /// disconnect when they see their own `device_id`.
    pub fn subscribe_revocations(&self) -> broadcast::Receiver<String> {
        self.revocations.subscribe()
    }

    /// Find devices whose `last_seen_at` is older than `days` and are not
    /// already revoked. Pure — does not mutate anything; callers (the
    /// scheduled sweep) decide whether/how to call `revoke` for each id.
    pub fn sweep_inactive(
        &self,
        devices: &[DeviceRecord],
        days: u32,
        now_rfc3339: &str,
    ) -> Vec<String> {
        let now = match chrono::DateTime::parse_from_rfc3339(now_rfc3339) {
            Ok(dt) => dt,
            Err(_) => return Vec::new(),
        };
        let cutoff = now - chrono::Duration::days(i64::from(days));

        devices
            .iter()
            .filter(|d| d.revoked_at.is_none())
            .filter_map(|d| {
                let last_seen = chrono::DateTime::parse_from_rfc3339(&d.last_seen_at).ok()?;
                if last_seen < cutoff {
                    Some(d.device_id.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

fn index_entry(record: &DeviceRecord) -> IndexEntry {
    IndexEntry {
        device_id: record.device_id.clone(),
        token_hash: record.token_hash.clone(),
        scopes: record.scopes.clone(),
        blocked: record.revoked_at.is_some(),
        expires_at: record.expires_at.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::devices::store::DeviceStore;
    use crate::web::devices::types::DevicePlatform;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    // Re-derive a hash locally too, so `authenticate`'s doc comment claim
    // ("hash the presented token") is exercised even if `store::hash_token`
    // changes shape; kept private, test-only sanity check.
    fn sha256_hex(raw: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(raw.as_bytes());
        hex::encode(hasher.finalize())
    }

    async fn test_registry() -> (DeviceRegistry, DeviceStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = DeviceStore::with_base_dir(dir.path().to_path_buf());
        let registry = DeviceRegistry::load(store.clone()).await.unwrap();
        (registry, store, dir)
    }

    #[tokio::test]
    async fn authenticate_succeeds_for_valid_token() {
        let (registry, store, _dir) = test_registry().await;
        let (record, token) = store
            .insert(
                "Phone".to_string(),
                DevicePlatform::Ios,
                vec![DeviceScope::Chat],
                None,
            )
            .unwrap();
        registry.refresh(&record.device_id).await.unwrap();

        let auth = registry.authenticate(&token).await.unwrap();
        assert_eq!(auth.device_id, record.device_id);
        assert_eq!(auth.scopes, vec![DeviceScope::Chat]);
    }

    #[tokio::test]
    async fn authenticate_rejects_unknown_token() {
        let (registry, _store, _dir) = test_registry().await;
        assert!(
            registry
                .authenticate("tcd_not-a-real-token")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn authenticate_hash_matches_store_hash_token() {
        // Confirms the registry hashes with the same function the store
        // persists with (no drift between the two hashing call sites).
        let (registry, store, _dir) = test_registry().await;
        let (record, token) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        registry.refresh(&record.device_id).await.unwrap();
        assert_eq!(sha256_hex(&token), record.token_hash);
        assert!(registry.authenticate(&token).await.is_some());
    }

    #[tokio::test]
    async fn authenticate_rejects_revoked_device() {
        let (registry, store, _dir) = test_registry().await;
        let (record, token) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        registry.refresh(&record.device_id).await.unwrap();
        assert!(registry.authenticate(&token).await.is_some());

        registry.revoke(&record.device_id).await.unwrap();
        assert!(registry.authenticate(&token).await.is_none());
    }

    #[tokio::test]
    async fn authenticate_rejects_expired_device() {
        let (registry, store, dir) = test_registry().await;
        let (record, token) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();

        // Manually set an already-past expiry via the store file (no public
        // "set expires_at" API is in scope for B1's registry surface).
        let raw_path = dir.path().join("devices.json");
        let raw = std::fs::read_to_string(&raw_path).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        value["devices"][0]["expires_at"] = serde_json::json!("2000-01-01T00:00:00+00:00");
        std::fs::write(&raw_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        registry.refresh(&record.device_id).await.unwrap();
        assert!(registry.authenticate(&token).await.is_none());
    }

    #[tokio::test]
    async fn revoke_broadcasts_device_id() {
        let (registry, store, _dir) = test_registry().await;
        let (record, _token) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        registry.refresh(&record.device_id).await.unwrap();

        let mut rx = registry.subscribe_revocations();
        registry.revoke(&record.device_id).await.unwrap();

        let revoked_id = rx.recv().await.unwrap();
        assert_eq!(revoked_id, record.device_id);
    }

    #[tokio::test]
    async fn touch_debounces_disk_flush() {
        let (registry, store, _dir) = test_registry().await;
        let (record, _token) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        registry.refresh(&record.device_id).await.unwrap();

        registry
            .touch(&record.device_id, "2020-01-01T00:00:00+00:00")
            .await
            .unwrap();
        // First touch always flushes (no prior last_flush entry).
        let after_first = store.get(&record.device_id).unwrap().unwrap();
        assert_eq!(after_first.last_seen_at, "2020-01-01T00:00:00+00:00");

        // A second touch immediately after should NOT flush yet (debounced).
        registry
            .touch(&record.device_id, "2021-01-01T00:00:00+00:00")
            .await
            .unwrap();
        let after_second = store.get(&record.device_id).unwrap().unwrap();
        assert_eq!(after_second.last_seen_at, "2020-01-01T00:00:00+00:00");

        // But flush_last_seen forces it through immediately.
        registry.flush_last_seen().await.unwrap();
        let after_flush = store.get(&record.device_id).unwrap().unwrap();
        assert_eq!(after_flush.last_seen_at, "2021-01-01T00:00:00+00:00");
    }

    #[tokio::test]
    async fn sweep_inactive_finds_stale_devices() {
        let (registry, _store, _dir) = test_registry().await;
        let stale = DeviceRecord {
            device_id: "stale".into(),
            name: "Stale".into(),
            platform: DevicePlatform::Ios,
            created_at: "2020-01-01T00:00:00+00:00".into(),
            last_seen_at: "2020-01-01T00:00:00+00:00".into(),
            token_hash: "h1".into(),
            token_prefix: "tcd_aaaa".into(),
            scopes: vec![],
            pubkey: None,
            apns: None,
            revoked_at: None,
            expires_at: None,
        };
        let fresh = DeviceRecord {
            device_id: "fresh".into(),
            last_seen_at: "2024-06-01T00:00:00+00:00".into(),
            ..stale.clone()
        };
        let already_revoked = DeviceRecord {
            device_id: "revoked".into(),
            revoked_at: Some("2020-01-01T00:00:00+00:00".into()),
            ..stale.clone()
        };

        let devices = vec![stale, fresh, already_revoked];
        let inactive = registry.sweep_inactive(&devices, 90, "2024-07-01T00:00:00+00:00");
        assert_eq!(inactive, vec!["stale".to_string()]);
    }
}
