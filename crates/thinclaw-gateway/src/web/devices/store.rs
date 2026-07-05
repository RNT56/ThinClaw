//! Persisted device store: `~/.thinclaw/devices.json`.
//!
//! Mirrors `crates/thinclaw-channels/src/pairing.rs` mechanics: fs4 file
//! locking around read-modify-write, tmp+rename atomic writes, a versioned
//! JSON envelope, and `with_base_dir` for tests.

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::FileExt;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::types::{DevicePlatform, DeviceRecord, DeviceScope};

/// Prefix registered in the `LeakDetector` scrub patterns (D-T1, D-N/logging
/// hygiene). Device tokens are `tcd_` + base64url(32 random bytes).
pub const DEVICE_TOKEN_PREFIX: &str = "tcd_";

const DEVICES_FILE_NAME: &str = "devices.json";
const DEVICES_FILE_VERSION: u8 = 1;

/// Error from device store operations.
#[derive(Debug, thiserror::Error)]
pub enum DeviceStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("device not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Serialize, Deserialize)]
struct DevicesFile {
    version: u8,
    devices: Vec<DeviceRecord>,
}

impl Default for DevicesFile {
    fn default() -> Self {
        Self {
            version: DEVICES_FILE_VERSION,
            devices: Vec::new(),
        }
    }
}

/// Current time as RFC 3339, for `expires_at`/`last_seen_at` comparisons.
pub fn now_iso() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    #[allow(clippy::cast_possible_wrap)]
    chrono::DateTime::from_timestamp(now.as_secs() as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| now.as_secs().to_string())
}

fn parse_json_or_default<T>(content: &str) -> Result<T, DeviceStoreError>
where
    T: serde::de::DeserializeOwned + Default,
{
    if content.trim().is_empty() {
        Ok(T::default())
    } else {
        serde_json::from_str(content).map_err(DeviceStoreError::from)
    }
}

/// Newly issued device credential. `token` is the raw, one-time-visible
/// bearer string; only `token_hash`/`token_prefix` are ever persisted.
pub struct IssuedToken {
    pub token: String,
    pub token_hash: String,
    pub token_prefix: String,
}

/// Generate a new `tcd_` device token: 32 CSPRNG bytes, base64url (no
/// padding), fixed prefix. Returns the raw token alongside its hash and
/// display prefix.
pub fn issue_token() -> IssuedToken {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    let encoded = base64_url_no_pad(&bytes);
    let token = format!("{DEVICE_TOKEN_PREFIX}{encoded}");
    let token_hash = hash_token(&token);
    let token_prefix = token.chars().take(8).collect();
    IssuedToken {
        token,
        token_hash,
        token_prefix,
    }
}

/// `hex(SHA-256(token))` — the only form a device token is ever persisted
/// or compared in.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn base64_url_no_pad(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Persisted store of paired devices.
#[derive(Debug, Clone)]
pub struct DeviceStore {
    base_dir: PathBuf,
}

impl DeviceStore {
    /// Default store rooted at `~/.thinclaw` (or `$THINCLAW_HOME`).
    pub fn new() -> Self {
        Self {
            base_dir: thinclaw_platform::resolve_thinclaw_home(),
        }
    }

    /// Store rooted at a custom directory (for tests).
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn path(&self) -> PathBuf {
        self.base_dir.join(DEVICES_FILE_NAME)
    }

    fn read_locked(&self) -> Result<(fs::File, DevicesFile), DeviceStoreError> {
        let path = self.path();
        fs::create_dir_all(
            path.parent()
                .expect("devices.json path always has a parent"),
        )?;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        file.lock_exclusive()?;
        let mut content = String::new();
        use std::io::Read as _;
        file.read_to_string(&mut content)?;
        let data: DevicesFile = parse_json_or_default(&content)?;
        Ok((file, data))
    }

    fn write_locked(
        &self,
        file: &mut fs::File,
        data: &DevicesFile,
    ) -> Result<(), DeviceStoreError> {
        let json = serde_json::to_string_pretty(data)?;
        // Atomic tmp+rename, matching pairing.rs's approach for the primary
        // store file (block-list/allow-list use tmp+rename too).
        let path = self.path();
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, &json)?;
        fs::rename(&tmp_path, &path)?;
        // Keep the locked fd's view in sync in case callers reuse it before
        // unlocking (mirrors pairing.rs write_pairing_file_locked, which
        // truncates + rewrites the same fd rather than swapping files).
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    /// Load all devices (no filtering).
    pub fn list(&self) -> Result<Vec<DeviceRecord>, DeviceStoreError> {
        let (file, data) = self.read_locked()?;
        FileExt::unlock(&file)?;
        Ok(data.devices)
    }

    /// Look up a single device by id.
    pub fn get(&self, device_id: &str) -> Result<Option<DeviceRecord>, DeviceStoreError> {
        let (file, data) = self.read_locked()?;
        FileExt::unlock(&file)?;
        Ok(data.devices.into_iter().find(|d| d.device_id == device_id))
    }

    /// Insert a newly paired device, issuing a fresh token. Returns the
    /// created record and the raw token (visible exactly once).
    pub fn insert(
        &self,
        name: String,
        platform: DevicePlatform,
        scopes: Vec<DeviceScope>,
        pubkey: Option<String>,
    ) -> Result<(DeviceRecord, String), DeviceStoreError> {
        let issued = issue_token();
        let now = now_iso();
        let record = DeviceRecord {
            device_id: Uuid::new_v4().to_string(),
            name,
            platform,
            created_at: now.clone(),
            last_seen_at: now,
            token_hash: issued.token_hash,
            token_prefix: issued.token_prefix,
            scopes,
            pubkey,
            apns: None,
            revoked_at: None,
            expires_at: None,
        };

        let (mut file, mut data) = self.read_locked()?;
        data.devices.push(record.clone());
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;

        Ok((record, issued.token))
    }

    /// Rename a device. Returns `NotFound` if the id does not exist.
    pub fn rename(
        &self,
        device_id: &str,
        new_name: &str,
    ) -> Result<DeviceRecord, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let record = data
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(device_id.to_string()))?;
        record.name = new_name.to_string();
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(updated)
    }

    /// Mark a device revoked (sets `revoked_at`; idempotent).
    pub fn revoke(&self, device_id: &str) -> Result<DeviceRecord, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let now = now_iso();
        let record = data
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(device_id.to_string()))?;
        if record.revoked_at.is_none() {
            record.revoked_at = Some(now);
        }
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(updated)
    }

    /// Issue a fresh token for an existing device, replacing the previous
    /// hash. Returns the updated record and the raw new token (visible
    /// exactly once).
    pub fn rotate(&self, device_id: &str) -> Result<(DeviceRecord, String), DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let issued = issue_token();
        let record = data
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(device_id.to_string()))?;
        record.token_hash = issued.token_hash;
        record.token_prefix = issued.token_prefix;
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok((updated, issued.token))
    }

    /// Update `last_seen_at`. Used by the registry's debounced flush.
    pub fn touch_last_seen(&self, device_id: &str, at: &str) -> Result<(), DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        if let Some(record) = data.devices.iter_mut().find(|d| d.device_id == device_id) {
            record.last_seen_at = at.to_string();
        }
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(())
    }

    /// Permanently delete a device record.
    pub fn delete(&self, device_id: &str) -> Result<bool, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let original_len = data.devices.len();
        data.devices.retain(|d| d.device_id != device_id);
        let removed = data.devices.len() != original_len;
        if removed {
            self.write_locked(&mut file, &data)?;
        }
        FileExt::unlock(&file)?;
        Ok(removed)
    }
}

impl Default for DeviceStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (DeviceStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = DeviceStore::with_base_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn issued_token_has_expected_shape() {
        let issued = issue_token();
        assert!(issued.token.starts_with(DEVICE_TOKEN_PREFIX));
        assert_eq!(issued.token_hash, hash_token(&issued.token));
        assert_eq!(
            issued.token_prefix,
            issued.token.chars().take(8).collect::<String>()
        );
        // base64url alphabet only, no padding.
        let encoded = &issued.token[DEVICE_TOKEN_PREFIX.len()..];
        assert!(!encoded.contains('='));
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn tokens_are_unique() {
        let a = issue_token();
        let b = issue_token();
        assert_ne!(a.token, b.token);
        assert_ne!(a.token_hash, b.token_hash);
    }

    #[test]
    fn insert_and_get_round_trip() {
        let (store, _dir) = test_store();
        let (record, token) = store
            .insert(
                "Phone".to_string(),
                DevicePlatform::Ios,
                DeviceScope::default_grant(),
                None,
            )
            .unwrap();
        assert!(token.starts_with(DEVICE_TOKEN_PREFIX));
        assert_eq!(record.token_hash, hash_token(&token));

        let fetched = store.get(&record.device_id).unwrap().unwrap();
        assert_eq!(fetched.device_id, record.device_id);
        assert_eq!(fetched.name, "Phone");
    }

    #[test]
    fn raw_token_never_persisted_to_disk() {
        let (store, dir) = test_store();
        let (_, token) = store
            .insert(
                "Phone".to_string(),
                DevicePlatform::Ios,
                DeviceScope::default_grant(),
                None,
            )
            .unwrap();

        let raw_file = fs::read_to_string(dir.path().join(DEVICES_FILE_NAME)).unwrap();
        assert!(!raw_file.contains(&token));
        // The persisted hash must be present, though.
        assert!(raw_file.contains(&hash_token(&token)));
    }

    #[test]
    fn rename_updates_name() {
        let (store, _dir) = test_store();
        let (record, _) = store
            .insert("Old".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        let updated = store.rename(&record.device_id, "New").unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(store.get(&record.device_id).unwrap().unwrap().name, "New");
    }

    #[test]
    fn rename_missing_device_errors() {
        let (store, _dir) = test_store();
        let err = store.rename("missing", "x").unwrap_err();
        assert!(matches!(err, DeviceStoreError::NotFound(_)));
    }

    #[test]
    fn revoke_sets_revoked_at_and_is_idempotent() {
        let (store, _dir) = test_store();
        let (record, _) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        assert!(record.revoked_at.is_none());

        let revoked = store.revoke(&record.device_id).unwrap();
        assert!(revoked.revoked_at.is_some());

        // Second revoke keeps the original timestamp rather than clobbering it.
        let first_ts = revoked.revoked_at.clone();
        let revoked_again = store.revoke(&record.device_id).unwrap();
        assert_eq!(revoked_again.revoked_at, first_ts);
    }

    #[test]
    fn rotate_issues_new_token_and_invalidates_old_hash() {
        let (store, _dir) = test_store();
        let (record, old_token) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();

        let (updated, new_token) = store.rotate(&record.device_id).unwrap();
        assert_ne!(new_token, old_token);
        assert_eq!(updated.token_hash, hash_token(&new_token));
        assert_ne!(updated.token_hash, hash_token(&old_token));
    }

    #[test]
    fn rotate_missing_device_errors() {
        let (store, _dir) = test_store();
        let err = store.rotate("missing").unwrap_err();
        assert!(matches!(err, DeviceStoreError::NotFound(_)));
    }

    #[test]
    fn delete_removes_record() {
        let (store, _dir) = test_store();
        let (record, _) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        assert!(store.delete(&record.device_id).unwrap());
        assert!(store.get(&record.device_id).unwrap().is_none());
        assert!(!store.delete(&record.device_id).unwrap());
    }

    #[test]
    fn list_returns_all_devices() {
        let (store, _dir) = test_store();
        store
            .insert("A".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        store
            .insert("B".to_string(), DevicePlatform::Macos, vec![], None)
            .unwrap();
        let all = store.list().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn touch_last_seen_updates_timestamp() {
        let (store, _dir) = test_store();
        let (record, _) = store
            .insert("Phone".to_string(), DevicePlatform::Ios, vec![], None)
            .unwrap();
        store
            .touch_last_seen(&record.device_id, "2099-01-01T00:00:00+00:00")
            .unwrap();
        let fetched = store.get(&record.device_id).unwrap().unwrap();
        assert_eq!(fetched.last_seen_at, "2099-01-01T00:00:00+00:00");
    }

    #[test]
    fn is_blocked_reflects_revocation_and_expiry() {
        let mut record = DeviceRecord {
            device_id: "d1".into(),
            name: "n".into(),
            platform: DevicePlatform::Ios,
            created_at: now_iso(),
            last_seen_at: now_iso(),
            token_hash: "h".into(),
            token_prefix: "tcd_abcd".into(),
            scopes: vec![],
            pubkey: None,
            apns: None,
            revoked_at: None,
            expires_at: None,
        };
        assert!(!record.is_blocked("2030-01-01T00:00:00+00:00"));

        record.expires_at = Some("2020-01-01T00:00:00+00:00".into());
        assert!(record.is_blocked("2030-01-01T00:00:00+00:00"));

        record.expires_at = None;
        record.revoked_at = Some(now_iso());
        assert!(record.is_blocked("2030-01-01T00:00:00+00:00"));
    }
}
