//! Persisted device store: `~/.thinclaw/devices.json`.
//!
//! Uses a stable sidecar lock around read-modify-write, durable atomic data
//! replacement, a bounded versioned JSON envelope, and `with_base_dir` for
//! tests.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::FileExt;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::types::{
    DeviceApnsRegistration, DeviceLiveActivityKind, DeviceLiveActivityToken, DevicePlatform,
    DeviceRecord, DeviceScope, MAX_LIVE_ACTIVITIES_PER_DEVICE,
};

/// Prefix registered in the `LeakDetector` scrub patterns (D-T1, D-N/logging
/// hygiene). Device tokens are `tcd_` + base64url(32 random bytes).
pub const DEVICE_TOKEN_PREFIX: &str = "tcd_";

const DEVICES_FILE_NAME: &str = "devices.json";
const DEVICES_LOCK_FILE_NAME: &str = "devices.lock";
const DEVICES_FILE_VERSION: u8 = 1;
const MAX_DEVICES_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_DEVICE_RECORDS: usize = 4_096;
const MAX_DEVICE_NAME_BYTES: usize = 256;
const MAX_DEVICE_PLATFORM_BYTES: usize = 64;
const MAX_DEVICE_PUBKEY_BYTES: usize = 3 * 1024;
const MAX_DEVICE_IDENTIFIER_BYTES: usize = 256;
const MAX_PUSH_TOKEN_BYTES: usize = 512;

/// Error from device store operations.
#[derive(Debug, thiserror::Error)]
pub enum DeviceStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("device not found: {0}")]
    NotFound(String),

    #[error("device revoked: {0}")]
    Revoked(String),

    #[error("invalid persisted device data: {0}")]
    InvalidData(String),
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

    fn lock_path(&self) -> PathBuf {
        self.base_dir.join(DEVICES_LOCK_FILE_NAME)
    }

    fn read_locked(&self) -> Result<(fs::File, DevicesFile), DeviceStoreError> {
        fs::create_dir_all(&self.base_dir)?;
        let base_metadata = fs::symlink_metadata(&self.base_dir)?;
        if base_metadata.file_type().is_symlink() || !base_metadata.is_dir() {
            return Err(DeviceStoreError::InvalidData(
                "device-store base path is not a real directory".to_string(),
            ));
        }

        let mut lock_options = fs::OpenOptions::new();
        lock_options
            .read(true)
            .write(true)
            .create(true)
            .truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            lock_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock_file = lock_options.open(self.lock_path())?;
        #[cfg(unix)]
        lock_file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        lock_file.lock_exclusive()?;

        let data = self.read_data_file()?;
        validate_devices_file(&data)?;
        Ok((lock_file, data))
    }

    fn write_locked(
        &self,
        _lock_file: &mut fs::File,
        data: &DevicesFile,
    ) -> Result<(), DeviceStoreError> {
        validate_devices_file(data)?;
        let json = serde_json::to_string_pretty(data)?;
        if json.len() > MAX_DEVICES_FILE_BYTES {
            return Err(DeviceStoreError::InvalidData(format!(
                "serialized device store exceeds the {MAX_DEVICES_FILE_BYTES}-byte limit"
            )));
        }
        let path = self.path();
        validate_existing_regular_file(&path)?;
        let tmp_path = self.base_dir.join(format!(
            ".{DEVICES_FILE_NAME}.{}.tmp",
            Uuid::new_v4().simple()
        ));
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let write_result = (|| -> std::io::Result<()> {
            let mut tmp = options.open(&tmp_path)?;
            tmp.write_all(json.as_bytes())?;
            tmp.sync_all()
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&tmp_path);
            return Err(error.into());
        }

        if let Err(error) = replace_data_file(&tmp_path, &path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(error.into());
        }
        sync_directory(&self.base_dir)?;
        Ok(())
    }

    fn read_data_file(&self) -> Result<DevicesFile, DeviceStoreError> {
        let path = self.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DevicesFile::default());
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_DEVICES_FILE_BYTES as u64
        {
            return Err(DeviceStoreError::InvalidData(format!(
                "{DEVICES_FILE_NAME} must be a regular file no larger than {MAX_DEVICES_FILE_BYTES} bytes"
            )));
        }

        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&path)?;
        #[cfg(unix)]
        file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
        let opened_metadata = file.metadata()?;
        if !opened_metadata.is_file() || opened_metadata.len() > MAX_DEVICES_FILE_BYTES as u64 {
            return Err(DeviceStoreError::InvalidData(
                "device store changed while it was being opened".to_string(),
            ));
        }
        let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
        Read::by_ref(&mut file)
            .take(MAX_DEVICES_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_DEVICES_FILE_BYTES {
            return Err(DeviceStoreError::InvalidData(format!(
                "{DEVICES_FILE_NAME} exceeds the {MAX_DEVICES_FILE_BYTES}-byte limit"
            )));
        }
        let content = String::from_utf8(bytes).map_err(|_| {
            DeviceStoreError::InvalidData(format!("{DEVICES_FILE_NAME} is not valid UTF-8"))
        })?;
        parse_json_or_default(&content)
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
            live_activities: std::collections::BTreeMap::new(),
            live_activity_start_token: None,
            parent_device_id: None,
            revoked_at: None,
            expires_at: None,
        };

        let (mut file, mut data) = self.read_locked()?;
        data.devices.push(record.clone());
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;

        Ok((record, issued.token))
    }

    /// Insert a *companion* device (milestone M4): a reduced-scope child minted
    /// by an already-paired `parent_device_id` (e.g. its watch). Fails with
    /// `NotFound` if the parent does not exist and `Revoked` if the parent is
    /// already revoked — a revoked parent must never spawn a live companion.
    /// Returns the created record and the raw token (visible exactly once).
    pub fn insert_companion(
        &self,
        parent_device_id: &str,
        name: String,
        platform: DevicePlatform,
        scopes: Vec<DeviceScope>,
    ) -> Result<(DeviceRecord, String), DeviceStoreError> {
        let issued = issue_token();
        let now = now_iso();

        let (mut file, mut data) = self.read_locked()?;
        // Validate the parent inside the same locked read-modify-write so the
        // check cannot race a concurrent parent revoke/delete.
        let parent = data
            .devices
            .iter()
            .find(|d| d.device_id == parent_device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(parent_device_id.to_string()))?;
        if parent.revoked_at.is_some() {
            return Err(DeviceStoreError::Revoked(parent_device_id.to_string()));
        }

        let record = DeviceRecord {
            device_id: Uuid::new_v4().to_string(),
            name,
            platform,
            created_at: now.clone(),
            last_seen_at: now,
            token_hash: issued.token_hash,
            token_prefix: issued.token_prefix,
            scopes,
            pubkey: None,
            apns: None,
            live_activities: std::collections::BTreeMap::new(),
            live_activity_start_token: None,
            parent_device_id: Some(parent_device_id.to_string()),
            revoked_at: None,
            expires_at: None,
        };
        data.devices.push(record.clone());
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;

        Ok((record, issued.token))
    }

    /// List the companion devices whose `parent_device_id == parent_device_id`.
    /// Returns an empty vec if the parent has no companions (or does not
    /// exist); companion lifecycle is validated by the caller.
    pub fn list_companions(
        &self,
        parent_device_id: &str,
    ) -> Result<Vec<DeviceRecord>, DeviceStoreError> {
        let (file, data) = self.read_locked()?;
        FileExt::unlock(&file)?;
        Ok(data
            .devices
            .into_iter()
            .filter(|d| d.parent_device_id.as_deref() == Some(parent_device_id))
            .collect())
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

    /// Mark a device revoked (sets `revoked_at`; idempotent). Also clears all
    /// push + Live Activity registrations in the *same* locked write, so a
    /// revoked device's stale tokens can never be pushed to (D-N /
    /// `docs/MOBILE_SECURITY.md` §8 hardening item 5 — "APNs registration
    /// deletion on revoke"). Clearing is unconditional (not gated on the
    /// idempotency guard) so a re-revoke still guarantees no push state
    /// lingers.
    ///
    /// **Cascade (milestone M4, D-K4):** revoking a device also revokes every
    /// companion whose `parent_device_id == device_id`, in the *same* locked
    /// write, so a parent revoke can never leave a live child token behind.
    /// Returns the primary (target) record; use [`revoke_cascade`] to also see
    /// the affected companion records (e.g. to broadcast/refresh each id).
    ///
    /// [`revoke_cascade`]: DeviceStore::revoke_cascade
    pub fn revoke(&self, device_id: &str) -> Result<DeviceRecord, DeviceStoreError> {
        let mut affected = self.revoke_cascade(device_id)?;
        // `revoke_cascade` returns the target record first, then companions.
        Ok(affected.remove(0))
    }

    /// Revoke `device_id` and cascade to all its companions in one locked
    /// write. Returns the affected records: the target first, then each
    /// companion (in stored order). Clearing push/Live Activity state is
    /// unconditional per record, matching [`revoke`]'s idempotency guarantee.
    ///
    /// [`revoke`]: DeviceStore::revoke
    pub fn revoke_cascade(&self, device_id: &str) -> Result<Vec<DeviceRecord>, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let now = now_iso();

        // The target must exist. Companions are best-effort: absent companions
        // simply don't contribute to the affected list.
        if !data.devices.iter().any(|d| d.device_id == device_id) {
            return Err(DeviceStoreError::NotFound(device_id.to_string()));
        }

        let mut affected = Vec::new();
        for record in data.devices.iter_mut() {
            let is_target = record.device_id == device_id;
            let is_companion = record.parent_device_id.as_deref() == Some(device_id);
            if !is_target && !is_companion {
                continue;
            }
            if record.revoked_at.is_none() {
                record.revoked_at = Some(now.clone());
            }
            record.apns = None;
            record.live_activities.clear();
            record.live_activity_start_token = None;
            if is_target {
                // Target first so callers can rely on affected[0] == target.
                affected.insert(0, record.clone());
            } else {
                affected.push(record.clone());
            }
        }

        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(affected)
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
        if record.revoked_at.is_some() {
            return Err(DeviceStoreError::Revoked(device_id.to_string()));
        }
        record.token_hash = issued.token_hash;
        record.token_prefix = issued.token_prefix;
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok((updated, issued.token))
    }

    /// Register (or replace) the device's APNs push token. Returns the
    /// updated record. `NotFound` if the id does not exist; `Revoked` if the
    /// device has been revoked — a revoked device must never be able to
    /// re-attach a push token (D-N, `docs/MOBILE_SECURITY.md` §8 item 5), and
    /// the check runs inside the same locked read-modify-write so it cannot
    /// race a concurrent `revoke`.
    pub fn set_push(
        &self,
        device_id: &str,
        device_token: String,
        environment: String,
    ) -> Result<DeviceRecord, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let now = now_iso();
        let record = data
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(device_id.to_string()))?;
        if record.revoked_at.is_some() {
            return Err(DeviceStoreError::Revoked(device_id.to_string()));
        }
        record.apns = Some(DeviceApnsRegistration {
            device_token,
            environment,
            updated_at: now,
        });
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(updated)
    }

    /// Clear the device's APNs push token. Idempotent (clearing an unset
    /// registration is a no-op that still succeeds).
    pub fn clear_push(&self, device_id: &str) -> Result<DeviceRecord, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let record = data
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(device_id.to_string()))?;
        record.apns = None;
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(updated)
    }

    /// Register (or replace) the Live Activity update token for `activity_id`.
    /// When the per-device cap ([`MAX_LIVE_ACTIVITIES_PER_DEVICE`]) would be
    /// exceeded by a *new* activity, the oldest entry (by `updated_at`, then
    /// `activity_id` for a stable tiebreak) is evicted first. Replacing an
    /// existing `activity_id` never evicts.
    pub fn set_live_activity(
        &self,
        device_id: &str,
        activity_id: &str,
        push_token: String,
        kind: DeviceLiveActivityKind,
        thread_id: Option<String>,
        job_id: Option<String>,
    ) -> Result<DeviceRecord, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let now = now_iso();
        let record = data
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(device_id.to_string()))?;
        if record.revoked_at.is_some() {
            return Err(DeviceStoreError::Revoked(device_id.to_string()));
        }

        let is_new = !record.live_activities.contains_key(activity_id);
        if is_new && record.live_activities.len() >= MAX_LIVE_ACTIVITIES_PER_DEVICE {
            // Evict the oldest by (updated_at, activity_id) so the write stays
            // bounded. `updated_at` is RFC 3339, so lexical order == time
            // order; the id tiebreak keeps eviction deterministic.
            if let Some(oldest_id) = record
                .live_activities
                .iter()
                .min_by(|a, b| {
                    a.1.updated_at
                        .cmp(&b.1.updated_at)
                        .then_with(|| a.0.cmp(b.0))
                })
                .map(|(id, _)| id.clone())
            {
                record.live_activities.remove(&oldest_id);
            }
        }

        record.live_activities.insert(
            activity_id.to_string(),
            DeviceLiveActivityToken {
                push_token,
                kind,
                thread_id,
                job_id,
                updated_at: now,
            },
        );
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(updated)
    }

    /// Remove the Live Activity token for `activity_id`. Idempotent.
    pub fn clear_live_activity(
        &self,
        device_id: &str,
        activity_id: &str,
    ) -> Result<DeviceRecord, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let record = data
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(device_id.to_string()))?;
        record.live_activities.remove(activity_id);
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(updated)
    }

    /// Register (or replace) the device's Live Activity push-to-start token.
    pub fn set_live_activity_start_token(
        &self,
        device_id: &str,
        push_token: String,
    ) -> Result<DeviceRecord, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let record = data
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(device_id.to_string()))?;
        if record.revoked_at.is_some() {
            return Err(DeviceStoreError::Revoked(device_id.to_string()));
        }
        record.live_activity_start_token = Some(push_token);
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(updated)
    }

    /// Clear the device's Live Activity push-to-start token. Idempotent.
    pub fn clear_live_activity_start_token(
        &self,
        device_id: &str,
    ) -> Result<DeviceRecord, DeviceStoreError> {
        let (mut file, mut data) = self.read_locked()?;
        let record = data
            .devices
            .iter_mut()
            .find(|d| d.device_id == device_id)
            .ok_or_else(|| DeviceStoreError::NotFound(device_id.to_string()))?;
        record.live_activity_start_token = None;
        let updated = record.clone();
        self.write_locked(&mut file, &data)?;
        FileExt::unlock(&file)?;
        Ok(updated)
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
        data.devices.retain(|d| {
            d.device_id != device_id && d.parent_device_id.as_deref() != Some(device_id)
        });
        let removed = data.devices.len() != original_len;
        if removed {
            self.write_locked(&mut file, &data)?;
        }
        FileExt::unlock(&file)?;
        Ok(removed)
    }
}

fn validate_devices_file(data: &DevicesFile) -> Result<(), DeviceStoreError> {
    use base64::Engine as _;
    use std::collections::HashSet;

    if data.version != DEVICES_FILE_VERSION {
        return Err(DeviceStoreError::InvalidData(format!(
            "unsupported devices.json version {} (expected {DEVICES_FILE_VERSION})",
            data.version
        )));
    }
    if data.devices.len() > MAX_DEVICE_RECORDS {
        return Err(DeviceStoreError::InvalidData(format!(
            "device count exceeds the {MAX_DEVICE_RECORDS}-record limit"
        )));
    }

    let mut ids = HashSet::with_capacity(data.devices.len());
    let mut token_hashes = HashSet::with_capacity(data.devices.len());
    for record in &data.devices {
        if !uuid::Uuid::parse_str(&record.device_id)
            .is_ok_and(|parsed| parsed.to_string() == record.device_id)
            || !ids.insert(&record.device_id)
        {
            return Err(DeviceStoreError::InvalidData(
                "device IDs must be unique UUIDs".to_string(),
            ));
        }
        if record.token_hash.len() != 64
            || !record
                .token_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !token_hashes.insert(&record.token_hash)
        {
            return Err(DeviceStoreError::InvalidData(
                "device token hashes must be unique SHA-256 hex strings".to_string(),
            ));
        }
        if !valid_bounded_text(&record.name, MAX_DEVICE_NAME_BYTES)
            || !valid_bounded_text(record.platform.as_str(), MAX_DEVICE_PLATFORM_BYTES)
            || record.token_prefix.len() != 8
            || !record.token_prefix.starts_with(DEVICE_TOKEN_PREFIX)
            || !record.token_prefix[DEVICE_TOKEN_PREFIX.len()..]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || record.live_activities.len() > MAX_LIVE_ACTIVITIES_PER_DEVICE
        {
            return Err(DeviceStoreError::InvalidData(
                "device record contains invalid or unbounded display/token metadata".to_string(),
            ));
        }
        let distinct_scopes: HashSet<_> = record.scopes.iter().copied().collect();
        if distinct_scopes.len() != record.scopes.len()
            || (record.parent_device_id.is_some()
                && record
                    .scopes
                    .iter()
                    .any(|scope| !matches!(scope, DeviceScope::Chat | DeviceScope::Approvals)))
        {
            return Err(DeviceStoreError::InvalidData(
                "device record contains duplicated or over-privileged scopes".to_string(),
            ));
        }
        if let Some(pubkey) = record.pubkey.as_deref() {
            let decoded = if valid_bounded_text(pubkey, MAX_DEVICE_PUBKEY_BYTES) {
                base64::engine::general_purpose::STANDARD
                    .decode(pubkey)
                    .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(pubkey))
                    .ok()
            } else {
                None
            };
            if decoded
                .as_deref()
                .is_none_or(|der| der.is_empty() || der.len() > 2_048 || der.first() != Some(&0x30))
            {
                return Err(DeviceStoreError::InvalidData(
                    "device public key is not bounded base64 SPKI DER".to_string(),
                ));
            }
        }
        if let Some(apns) = record.apns.as_ref()
            && (!valid_bounded_text(&apns.device_token, MAX_PUSH_TOKEN_BYTES)
                || !matches!(apns.environment.as_str(), "development" | "production")
                || !valid_timestamp(&apns.updated_at))
        {
            return Err(DeviceStoreError::InvalidData(
                "device APNs registration is invalid or unbounded".to_string(),
            ));
        }
        if record
            .live_activity_start_token
            .as_deref()
            .is_some_and(|token| !valid_bounded_text(token, MAX_PUSH_TOKEN_BYTES))
        {
            return Err(DeviceStoreError::InvalidData(
                "device Live Activity start token is invalid or unbounded".to_string(),
            ));
        }
        for (activity_id, activity) in &record.live_activities {
            let invalid_association = match activity.kind {
                DeviceLiveActivityKind::AgentRun => activity.job_id.is_some(),
                DeviceLiveActivityKind::Job => activity.thread_id.is_some(),
            };
            if !valid_bounded_text(activity_id, MAX_DEVICE_IDENTIFIER_BYTES)
                || !valid_bounded_text(&activity.push_token, MAX_PUSH_TOKEN_BYTES)
                || activity
                    .thread_id
                    .as_deref()
                    .is_some_and(|id| !valid_bounded_text(id, MAX_DEVICE_IDENTIFIER_BYTES))
                || activity
                    .job_id
                    .as_deref()
                    .is_some_and(|id| !valid_bounded_text(id, MAX_DEVICE_IDENTIFIER_BYTES))
                || invalid_association
                || !valid_timestamp(&activity.updated_at)
            {
                return Err(DeviceStoreError::InvalidData(
                    "device Live Activity registration is invalid or unbounded".to_string(),
                ));
            }
        }
        for timestamp in [
            Some(record.created_at.as_str()),
            Some(record.last_seen_at.as_str()),
            record.revoked_at.as_deref(),
            record.expires_at.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !valid_timestamp(timestamp) {
                return Err(DeviceStoreError::InvalidData(
                    "device record contains an invalid timestamp".to_string(),
                ));
            }
        }
        if let Some(parent_id) = record.parent_device_id.as_deref()
            && uuid::Uuid::parse_str(parent_id).is_err()
        {
            return Err(DeviceStoreError::InvalidData(
                "companion parent device ID is not a UUID".to_string(),
            ));
        }
    }
    for record in &data.devices {
        if let Some(parent_id) = record.parent_device_id.as_deref() {
            let Some(parent) = data
                .devices
                .iter()
                .find(|candidate| candidate.device_id == parent_id)
            else {
                return Err(DeviceStoreError::InvalidData(
                    "companion device references a missing parent".to_string(),
                ));
            };
            if parent.device_id == record.device_id
                || parent.parent_device_id.is_some()
                || (parent.revoked_at.is_some() && record.revoked_at.is_none())
            {
                return Err(DeviceStoreError::InvalidData(
                    "companion device has an invalid or revoked parent relationship".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn valid_bounded_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn valid_timestamp(value: &str) -> bool {
    value.len() <= 64 && chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

fn validate_existing_regular_file(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            std::io::Error::other("device-store target is not a regular file"),
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
fn replace_data_file(staged: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(staged, target)
}

#[cfg(windows)]
fn replace_data_file(staged: &Path, target: &Path) -> std::io::Result<()> {
    let backup = target.with_extension(format!("json.{}.bak", Uuid::new_v4().simple()));
    let had_target = target.exists();
    if had_target {
        fs::rename(target, &backup)?;
    }
    if let Err(error) = fs::rename(staged, target) {
        if had_target {
            let _ = fs::rename(&backup, target);
        }
        return Err(error);
    }
    if had_target {
        fs::remove_file(backup)?;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn replace_data_file(staged: &Path, target: &Path) -> std::io::Result<()> {
    if target.exists() {
        fs::remove_file(target)?;
    }
    fs::rename(staged, target)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
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

    fn insert_phone(store: &DeviceStore) -> DeviceRecord {
        store
            .insert(
                "Phone".to_string(),
                DevicePlatform::Ios,
                DeviceScope::default_grant(),
                None,
            )
            .unwrap()
            .0
    }

    #[test]
    fn set_and_clear_push_round_trips() {
        let (store, _dir) = test_store();
        let record = insert_phone(&store);
        assert!(record.apns.is_none());

        let updated = store
            .set_push(
                &record.device_id,
                "apns-device-token".to_string(),
                "production".to_string(),
            )
            .unwrap();
        let apns = updated.apns.expect("apns registration present");
        assert_eq!(apns.device_token, "apns-device-token");
        assert_eq!(apns.environment, "production");
        assert!(!apns.updated_at.is_empty());

        // Persisted and reloadable.
        let fetched = store.get(&record.device_id).unwrap().unwrap();
        assert_eq!(
            fetched.apns.as_ref().map(|a| a.device_token.as_str()),
            Some("apns-device-token")
        );

        let cleared = store.clear_push(&record.device_id).unwrap();
        assert!(cleared.apns.is_none());
        // Clearing again is a no-op that still succeeds (idempotent).
        assert!(store.clear_push(&record.device_id).unwrap().apns.is_none());
    }

    #[test]
    fn set_push_hash_at_rest_invariant_untouched() {
        // Registering a push token must not disturb the token hashing
        // invariants (no raw device *auth* token leaks; the stored hash is
        // still hex(SHA-256(token))).
        let (store, dir) = test_store();
        let (record, token) = store
            .insert(
                "Phone".to_string(),
                DevicePlatform::Ios,
                DeviceScope::default_grant(),
                None,
            )
            .unwrap();
        store
            .set_push(
                &record.device_id,
                "apns-device-token".to_string(),
                "development".to_string(),
            )
            .unwrap();

        let raw = fs::read_to_string(dir.path().join(DEVICES_FILE_NAME)).unwrap();
        assert!(!raw.contains(&token), "raw auth token must never persist");
        assert!(raw.contains(&hash_token(&token)));
    }

    #[test]
    fn set_push_missing_device_errors() {
        let (store, _dir) = test_store();
        let err = store
            .set_push("missing", "t".to_string(), "production".to_string())
            .unwrap_err();
        assert!(matches!(err, DeviceStoreError::NotFound(_)));
    }

    #[test]
    fn set_and_clear_live_activity_round_trips() {
        let (store, _dir) = test_store();
        let record = insert_phone(&store);

        let updated = store
            .set_live_activity(
                &record.device_id,
                "run-1",
                "la-token-1".to_string(),
                DeviceLiveActivityKind::AgentRun,
                Some("thread-1".to_string()),
                None,
            )
            .unwrap();
        let entry = updated.live_activities.get("run-1").unwrap();
        assert_eq!(entry.push_token, "la-token-1");
        assert_eq!(entry.kind, DeviceLiveActivityKind::AgentRun);
        assert_eq!(entry.thread_id.as_deref(), Some("thread-1"));

        // Replacing the same activity_id updates in place, no duplicate.
        let updated = store
            .set_live_activity(
                &record.device_id,
                "run-1",
                "la-token-2".to_string(),
                DeviceLiveActivityKind::Job,
                None,
                Some("job-9".to_string()),
            )
            .unwrap();
        assert_eq!(updated.live_activities.len(), 1);
        assert_eq!(updated.live_activities["run-1"].push_token, "la-token-2");
        assert_eq!(
            updated.live_activities["run-1"].kind,
            DeviceLiveActivityKind::Job
        );
        // Replacement swaps the association too: thread cleared, job set.
        assert_eq!(updated.live_activities["run-1"].thread_id, None);
        assert_eq!(
            updated.live_activities["run-1"].job_id.as_deref(),
            Some("job-9")
        );

        let cleared = store
            .clear_live_activity(&record.device_id, "run-1")
            .unwrap();
        assert!(cleared.live_activities.is_empty());
        // Idempotent clear.
        assert!(
            store
                .clear_live_activity(&record.device_id, "run-1")
                .unwrap()
                .live_activities
                .is_empty()
        );
    }

    #[test]
    fn live_activities_cap_evicts_oldest() {
        let (store, _dir) = test_store();
        let record = insert_phone(&store);

        // Fill to exactly the cap, giving each a distinct (increasing)
        // updated_at so eviction order is deterministic. We set updated_at by
        // rewriting the file between inserts is overkill; instead rely on the
        // store's own now_iso — which may collide within the same second — so
        // manually normalize timestamps afterward to make "oldest" unambiguous.
        for i in 0..MAX_LIVE_ACTIVITIES_PER_DEVICE {
            store
                .set_live_activity(
                    &record.device_id,
                    &format!("run-{i}"),
                    format!("tok-{i}"),
                    DeviceLiveActivityKind::AgentRun,
                    None,
                    None,
                )
                .unwrap();
        }
        // Rewrite updated_at so run-0 is unambiguously the oldest and the rest
        // strictly increase.
        let raw_path = _dir.path().join(DEVICES_FILE_NAME);
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&raw_path).unwrap()).unwrap();
        for i in 0..MAX_LIVE_ACTIVITIES_PER_DEVICE {
            value["devices"][0]["live_activities"][format!("run-{i}")]["updated_at"] =
                serde_json::json!(format!("2024-01-01T00:00:{:02}+00:00", i));
        }
        fs::write(&raw_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        assert_eq!(
            store
                .get(&record.device_id)
                .unwrap()
                .unwrap()
                .live_activities
                .len(),
            MAX_LIVE_ACTIVITIES_PER_DEVICE
        );

        // One more *new* activity evicts the oldest (run-0) and keeps the cap.
        let updated = store
            .set_live_activity(
                &record.device_id,
                "run-new",
                "tok-new".to_string(),
                DeviceLiveActivityKind::Job,
                None,
                None,
            )
            .unwrap();
        assert_eq!(
            updated.live_activities.len(),
            MAX_LIVE_ACTIVITIES_PER_DEVICE
        );
        assert!(!updated.live_activities.contains_key("run-0"));
        assert!(updated.live_activities.contains_key("run-new"));
    }

    #[test]
    fn set_and_clear_live_activity_start_token_round_trips() {
        let (store, _dir) = test_store();
        let record = insert_phone(&store);

        let updated = store
            .set_live_activity_start_token(&record.device_id, "start-token".to_string())
            .unwrap();
        assert_eq!(
            updated.live_activity_start_token.as_deref(),
            Some("start-token")
        );

        let cleared = store
            .clear_live_activity_start_token(&record.device_id)
            .unwrap();
        assert!(cleared.live_activity_start_token.is_none());
    }

    #[test]
    fn revoke_clears_all_push_registrations_atomically() {
        let (store, _dir) = test_store();
        let record = insert_phone(&store);
        store
            .set_push(
                &record.device_id,
                "apns-device-token".to_string(),
                "production".to_string(),
            )
            .unwrap();
        store
            .set_live_activity(
                &record.device_id,
                "run-1",
                "la-token".to_string(),
                DeviceLiveActivityKind::AgentRun,
                None,
                None,
            )
            .unwrap();
        store
            .set_live_activity_start_token(&record.device_id, "start-token".to_string())
            .unwrap();

        let revoked = store.revoke(&record.device_id).unwrap();
        assert!(revoked.revoked_at.is_some());
        assert!(revoked.apns.is_none());
        assert!(revoked.live_activities.is_empty());
        assert!(revoked.live_activity_start_token.is_none());

        // Persisted, not just the returned copy.
        let fetched = store.get(&record.device_id).unwrap().unwrap();
        assert!(fetched.apns.is_none());
        assert!(fetched.live_activities.is_empty());
        assert!(fetched.live_activity_start_token.is_none());
    }

    #[test]
    fn set_push_on_revoked_device_is_rejected() {
        // R2: a revoked device must never be able to re-attach a push token.
        let (store, _dir) = test_store();
        let record = insert_phone(&store);
        store.revoke(&record.device_id).unwrap();

        let err = store
            .set_push(
                &record.device_id,
                "apns-device-token".to_string(),
                "production".to_string(),
            )
            .unwrap_err();
        assert!(matches!(err, DeviceStoreError::Revoked(_)));
        // Nothing was persisted onto the revoked record.
        let fetched = store.get(&record.device_id).unwrap().unwrap();
        assert!(fetched.apns.is_none());
    }

    #[test]
    fn set_live_activity_on_revoked_device_is_rejected() {
        let (store, _dir) = test_store();
        let record = insert_phone(&store);
        store.revoke(&record.device_id).unwrap();

        let err = store
            .set_live_activity(
                &record.device_id,
                "run-1",
                "la-token".to_string(),
                DeviceLiveActivityKind::AgentRun,
                None,
                None,
            )
            .unwrap_err();
        assert!(matches!(err, DeviceStoreError::Revoked(_)));
        assert!(
            store
                .get(&record.device_id)
                .unwrap()
                .unwrap()
                .live_activities
                .is_empty()
        );
    }

    #[test]
    fn set_live_activity_start_token_on_revoked_device_is_rejected() {
        let (store, _dir) = test_store();
        let record = insert_phone(&store);
        store.revoke(&record.device_id).unwrap();

        let err = store
            .set_live_activity_start_token(&record.device_id, "start-token".to_string())
            .unwrap_err();
        assert!(matches!(err, DeviceStoreError::Revoked(_)));
        assert!(
            store
                .get(&record.device_id)
                .unwrap()
                .unwrap()
                .live_activity_start_token
                .is_none()
        );
    }

    #[test]
    fn apns_field_reads_legacy_null_placeholder() {
        // Older devices.json files may carry `"apns": null` (or omit it) from
        // the B1 placeholder era. Both must still deserialize into `None`.
        let (store, dir) = test_store();
        let record = insert_phone(&store);
        let raw_path = dir.path().join(DEVICES_FILE_NAME);
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&raw_path).unwrap()).unwrap();
        value["devices"][0]["apns"] = serde_json::Value::Null;
        fs::write(&raw_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let fetched = store.get(&record.device_id).unwrap().unwrap();
        assert!(fetched.apns.is_none());
    }

    #[test]
    fn insert_companion_links_parent_and_reduced_scopes() {
        let (store, _dir) = test_store();
        let parent = insert_phone(&store);

        let (companion, token) = store
            .insert_companion(
                &parent.device_id,
                "Watch".to_string(),
                DevicePlatform::Watchos,
                DeviceScope::companion_grant(),
            )
            .unwrap();

        assert!(token.starts_with(DEVICE_TOKEN_PREFIX));
        assert_eq!(
            companion.parent_device_id.as_deref(),
            Some(parent.device_id.as_str())
        );
        assert!(companion.is_companion());
        assert_eq!(companion.platform, DevicePlatform::Watchos);
        assert_eq!(
            companion.scopes,
            vec![DeviceScope::Chat, DeviceScope::Approvals]
        );
        // Companion is a distinct record, persisted.
        assert_ne!(companion.device_id, parent.device_id);
        assert!(store.get(&companion.device_id).unwrap().is_some());
    }

    #[test]
    fn insert_companion_rejects_missing_or_revoked_parent() {
        let (store, _dir) = test_store();

        let err = store
            .insert_companion(
                "missing-parent",
                "Watch".to_string(),
                DevicePlatform::Watchos,
                DeviceScope::companion_grant(),
            )
            .unwrap_err();
        assert!(matches!(err, DeviceStoreError::NotFound(_)));

        let parent = insert_phone(&store);
        store.revoke(&parent.device_id).unwrap();
        let err = store
            .insert_companion(
                &parent.device_id,
                "Watch".to_string(),
                DevicePlatform::Watchos,
                DeviceScope::companion_grant(),
            )
            .unwrap_err();
        assert!(matches!(err, DeviceStoreError::Revoked(_)));
    }

    #[test]
    fn list_companions_filters_by_parent() {
        let (store, _dir) = test_store();
        let parent_a = insert_phone(&store);
        let parent_b = insert_phone(&store);

        let (c1, _) = store
            .insert_companion(
                &parent_a.device_id,
                "Watch A1".to_string(),
                DevicePlatform::Watchos,
                DeviceScope::companion_grant(),
            )
            .unwrap();
        store
            .insert_companion(
                &parent_b.device_id,
                "Watch B1".to_string(),
                DevicePlatform::Watchos,
                DeviceScope::companion_grant(),
            )
            .unwrap();

        let a_companions = store.list_companions(&parent_a.device_id).unwrap();
        assert_eq!(a_companions.len(), 1);
        assert_eq!(a_companions[0].device_id, c1.device_id);
        // The top-level parents themselves are not companions of anyone.
        assert!(
            store
                .list_companions(&parent_a.device_id)
                .unwrap()
                .iter()
                .all(|d| d.device_id != parent_a.device_id)
        );
    }

    #[test]
    fn revoke_cascades_to_companions() {
        let (store, _dir) = test_store();
        let parent = insert_phone(&store);
        let (companion, _) = store
            .insert_companion(
                &parent.device_id,
                "Watch".to_string(),
                DevicePlatform::Watchos,
                DeviceScope::companion_grant(),
            )
            .unwrap();
        // Give the companion a push token so we can assert it is cleared.
        store
            .set_push(
                &companion.device_id,
                "apns-tok".to_string(),
                "production".to_string(),
            )
            .unwrap();

        let affected = store.revoke_cascade(&parent.device_id).unwrap();
        // Target first, then the companion.
        assert_eq!(affected.len(), 2);
        assert_eq!(affected[0].device_id, parent.device_id);
        assert_eq!(affected[1].device_id, companion.device_id);

        let fetched_parent = store.get(&parent.device_id).unwrap().unwrap();
        let fetched_companion = store.get(&companion.device_id).unwrap().unwrap();
        assert!(fetched_parent.revoked_at.is_some());
        assert!(fetched_companion.revoked_at.is_some());
        assert!(
            fetched_companion.apns.is_none(),
            "cascade clears companion push state"
        );
    }

    #[test]
    fn revoke_of_companion_does_not_touch_parent() {
        let (store, _dir) = test_store();
        let parent = insert_phone(&store);
        let (companion, _) = store
            .insert_companion(
                &parent.device_id,
                "Watch".to_string(),
                DevicePlatform::Watchos,
                DeviceScope::companion_grant(),
            )
            .unwrap();

        store.revoke(&companion.device_id).unwrap();
        assert!(
            store
                .get(&companion.device_id)
                .unwrap()
                .unwrap()
                .revoked_at
                .is_some()
        );
        assert!(
            store
                .get(&parent.device_id)
                .unwrap()
                .unwrap()
                .revoked_at
                .is_none(),
            "revoking a companion must not revoke its parent"
        );
    }

    #[test]
    fn deleting_parent_also_deletes_companions() {
        let (store, _dir) = test_store();
        let parent = insert_phone(&store);
        let (companion, _) = store
            .insert_companion(
                &parent.device_id,
                "Watch".to_string(),
                DevicePlatform::Watchos,
                DeviceScope::companion_grant(),
            )
            .unwrap();

        assert!(store.delete(&parent.device_id).unwrap());
        assert!(store.get(&parent.device_id).unwrap().is_none());
        assert!(store.get(&companion.device_id).unwrap().is_none());
    }

    #[test]
    fn concurrent_writers_do_not_lose_device_records() {
        const WRITERS: usize = 16;
        let (store, _dir) = test_store();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(WRITERS));
        let mut workers = Vec::new();
        for index in 0..WRITERS {
            let store = store.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                store
                    .insert(
                        format!("Device {index}"),
                        DevicePlatform::Ios,
                        vec![DeviceScope::Chat],
                        None,
                    )
                    .unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }

        let records = store.list().unwrap();
        assert_eq!(records.len(), WRITERS);
        let unique_ids: std::collections::HashSet<_> =
            records.iter().map(|record| &record.device_id).collect();
        assert_eq!(unique_ids.len(), WRITERS);
    }

    #[test]
    fn unsupported_store_version_fails_closed() {
        let (store, dir) = test_store();
        fs::write(
            dir.path().join(DEVICES_FILE_NAME),
            r#"{"version":255,"devices":[]}"#,
        )
        .unwrap();

        assert!(matches!(
            store.list(),
            Err(DeviceStoreError::InvalidData(_))
        ));
    }

    #[test]
    fn unbounded_persisted_push_metadata_fails_closed() {
        let (store, dir) = test_store();
        insert_phone(&store);
        let path = dir.path().join(DEVICES_FILE_NAME);
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        value["devices"][0]["live_activity_start_token"] =
            serde_json::Value::String("x".repeat(MAX_PUSH_TOKEN_BYTES + 1));
        fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        assert!(matches!(
            store.list(),
            Err(DeviceStoreError::InvalidData(_))
        ));
    }

    #[test]
    fn overprivileged_persisted_companion_fails_closed() {
        let (store, dir) = test_store();
        let parent = insert_phone(&store);
        store
            .insert_companion(
                &parent.device_id,
                "Watch".to_string(),
                DevicePlatform::Watchos,
                DeviceScope::companion_grant(),
            )
            .unwrap();
        let path = dir.path().join(DEVICES_FILE_NAME);
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        value["devices"][1]["scopes"] = serde_json::json!(["chat", "jobs:read"]);
        fs::write(path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        assert!(matches!(
            store.list(),
            Err(DeviceStoreError::InvalidData(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_device_store_is_rejected() {
        use std::os::unix::fs::symlink;

        let (store, dir) = test_store();
        let outside = dir.path().join("outside.json");
        fs::write(&outside, r#"{"version":1,"devices":[]}"#).unwrap();
        symlink(&outside, dir.path().join(DEVICES_FILE_NAME)).unwrap();

        assert!(matches!(
            store.list(),
            Err(DeviceStoreError::InvalidData(_))
        ));
    }

    #[test]
    fn parent_device_id_defaults_for_legacy_rows() {
        // A legacy devices.json row written before companions existed omits
        // `parent_device_id`; it must deserialize to None (top-level device).
        let (store, dir) = test_store();
        let record = insert_phone(&store);
        let raw_path = dir.path().join(DEVICES_FILE_NAME);
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&raw_path).unwrap()).unwrap();
        value["devices"][0]
            .as_object_mut()
            .unwrap()
            .remove("parent_device_id");
        fs::write(&raw_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let fetched = store.get(&record.device_id).unwrap().unwrap();
        assert!(fetched.parent_device_id.is_none());
        assert!(!fetched.is_companion());
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
            live_activities: std::collections::BTreeMap::new(),
            live_activity_start_token: None,
            parent_device_id: None,
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
