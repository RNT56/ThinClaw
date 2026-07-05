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
use std::sync::{Arc, Mutex};
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
    /// Full device records keyed by `device_id`, kept in sync with the store
    /// on every mutation (`load`/`refresh`). The first-party push notifier
    /// reads this via [`DeviceRegistry::snapshot`] on the SSE hot path so it
    /// never touches disk (and never takes the store's exclusive file lock)
    /// per event.
    by_id: Arc<RwLock<HashMap<String, DeviceRecord>>>,
    last_seen: Arc<RwLock<LastSeenState>>,
    revocations: broadcast::Sender<String>,
    /// Per-device count of live SSE/WS streams currently open for that device
    /// principal. The first-party push notifier reads this to suppress Alert
    /// pushes to a device that is already watching events in-app (D-N1 local
    /// rewrite is moot when the app is foregrounded and streaming).
    active_streams: Arc<Mutex<HashMap<String, u32>>>,
}

/// RAII guard returned by [`DeviceRegistry::stream_opened`]. Increments the
/// device's active-stream count on creation and decrements it on `Drop`, so a
/// stream handler simply holds the guard for the connection's lifetime and the
/// count self-heals on disconnect, panic, or task cancellation.
pub struct StreamGuard {
    device_id: String,
    active_streams: Arc<Mutex<HashMap<String, u32>>>,
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.active_streams.lock()
            && let Some(count) = map.get_mut(&self.device_id)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.device_id);
            }
        }
    }
}

impl DeviceRegistry {
    /// Build a registry backed by `store`, loading its current contents
    /// into memory immediately.
    pub async fn load(store: DeviceStore) -> Result<Self, DeviceStoreError> {
        let store = Arc::new(store);
        let records = store.list()?;
        let mut by_hash = HashMap::with_capacity(records.len());
        let mut by_id = HashMap::with_capacity(records.len());
        for record in &records {
            by_hash.insert(record.token_hash.clone(), index_entry(record));
            by_id.insert(record.device_id.clone(), record.clone());
        }

        let (tx, _rx) = broadcast::channel(REVOCATION_CHANNEL_CAPACITY);

        Ok(Self {
            store,
            by_hash: Arc::new(RwLock::new(by_hash)),
            by_id: Arc::new(RwLock::new(by_id)),
            last_seen: Arc::new(RwLock::new(LastSeenState {
                pending: HashMap::new(),
                last_flush: HashMap::new(),
            })),
            revocations: tx,
            active_streams: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Read-only handle to the backing store, so runtime consumers (e.g. the
    /// first-party push notifier) can list devices and prune stale push
    /// registrations without reaching around the registry.
    pub fn store(&self) -> &DeviceStore {
        &self.store
    }

    /// Re-read a single device from the store and refresh its index entries.
    /// Callers should invoke this after any store mutation that changes a
    /// device's token/scopes/revocation state (insert/rotate/revoke) *or* its
    /// push/live-activity registrations, so the notifier's [`snapshot`] never
    /// serves a stale (or revoked) push registration.
    ///
    /// [`snapshot`]: DeviceRegistry::snapshot
    pub async fn refresh(&self, device_id: &str) -> Result<(), DeviceStoreError> {
        let record = self.store.get(device_id)?;
        {
            let mut by_hash = self.by_hash.write().await;
            by_hash.retain(|_, entry| entry.device_id != device_id);
            if let Some(record) = &record {
                by_hash.insert(record.token_hash.clone(), index_entry(record));
            }
        }
        {
            let mut by_id = self.by_id.write().await;
            match &record {
                Some(record) => {
                    by_id.insert(device_id.to_string(), record.clone());
                }
                None => {
                    by_id.remove(device_id);
                }
            }
        }
        Ok(())
    }

    /// Cloned snapshot of every device record currently in the in-memory
    /// index. Used by the first-party push notifier so its per-SSE-event
    /// device scan never does file I/O or takes the store's exclusive file
    /// lock. The index is refreshed on every store mutation, so this reflects
    /// registration/revocation changes without reading disk.
    pub async fn snapshot(&self) -> Vec<DeviceRecord> {
        self.by_id.read().await.values().cloned().collect()
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

    /// Record that a device principal has opened a live SSE/WS stream. Returns
    /// a [`StreamGuard`] the handler must hold for the connection's lifetime;
    /// dropping it decrements the count. While the count is non-zero the push
    /// notifier suppresses Alert pushes to this device (it is watching in-app).
    pub fn stream_opened(&self, device_id: &str) -> StreamGuard {
        if let Ok(mut map) = self.active_streams.lock() {
            *map.entry(device_id.to_string()).or_insert(0) += 1;
        }
        StreamGuard {
            device_id: device_id.to_string(),
            active_streams: Arc::clone(&self.active_streams),
        }
    }

    /// True if `device_id` currently has at least one live SSE/WS stream open.
    /// Read by the push notifier to decide Alert-push suppression (Live
    /// Activity updates are never suppressed).
    pub fn has_active_stream(&self, device_id: &str) -> bool {
        self.active_streams
            .lock()
            .map(|map| map.get(device_id).is_some_and(|&c| c > 0))
            .unwrap_or(false)
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
    async fn stream_guard_tracks_and_releases_active_streams() {
        let (registry, _store, _dir) = test_registry().await;
        assert!(!registry.has_active_stream("dev-1"));

        let g1 = registry.stream_opened("dev-1");
        assert!(registry.has_active_stream("dev-1"));

        // A second concurrent stream keeps it active until both drop.
        let g2 = registry.stream_opened("dev-1");
        assert!(registry.has_active_stream("dev-1"));
        drop(g1);
        assert!(registry.has_active_stream("dev-1"));
        drop(g2);
        assert!(!registry.has_active_stream("dev-1"));

        // Distinct devices are tracked independently.
        let g3 = registry.stream_opened("dev-2");
        assert!(registry.has_active_stream("dev-2"));
        assert!(!registry.has_active_stream("dev-1"));
        drop(g3);
        assert!(!registry.has_active_stream("dev-2"));
    }

    #[tokio::test]
    async fn snapshot_reflects_store_mutations_after_refresh() {
        let (registry, store, _dir) = test_registry().await;
        assert!(registry.snapshot().await.is_empty());

        let (record, _token) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        // Not visible until refreshed (snapshot is the in-memory index).
        assert!(registry.snapshot().await.is_empty());
        registry.refresh(&record.device_id).await.unwrap();

        let snap = registry.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].device_id, record.device_id);
        assert!(snap[0].apns.is_none());

        // A push registration shows up in the snapshot after refresh.
        store
            .set_push(
                &record.device_id,
                "apns-tok".to_string(),
                "production".to_string(),
            )
            .unwrap();
        registry.refresh(&record.device_id).await.unwrap();
        let snap = registry.snapshot().await;
        assert_eq!(
            snap[0].apns.as_ref().map(|a| a.device_token.as_str()),
            Some("apns-tok")
        );

        // Revocation is reflected too.
        registry.revoke(&record.device_id).await.unwrap();
        let snap = registry.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert!(snap[0].revoked_at.is_some());
        assert!(snap[0].apns.is_none(), "revoke clears push state");
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
            live_activities: std::collections::BTreeMap::new(),
            live_activity_start_token: None,
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
