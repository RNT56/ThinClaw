//! Pairing store: pending requests, allowFrom list, and blockFrom list.
//!
//! Stored in ~/.thinclaw/{channel}-pairing.json, {channel}-allowFrom.json,
//! and {channel}-blockFrom.json.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::FileExt;
use rand::RngExt;
use serde::{Deserialize, Serialize};

const PAIRING_CODE_LENGTH: usize = 8;
const PAIRING_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
/// TTL for pending pairing requests (minutes, not hours - reduces brute-force window).
const PAIRING_PENDING_TTL_SECS: u64 = 15 * 60;
const PAIRING_PENDING_MAX: usize = 3;
/// Max failed approve attempts per channel before rate limit kicks in.
const PAIRING_APPROVE_RATE_LIMIT: usize = 10;
/// Time window for rate limit (seconds).
const PAIRING_APPROVE_RATE_WINDOW_SECS: u64 = 5 * 60;
const MAX_PAIRING_FILE_BYTES: usize = 1024 * 1024;
const MAX_PAIRING_ID_BYTES: usize = 1024;
const MAX_PAIRING_META_BYTES: usize = 256 * 1024;
const MAX_ACCESS_ENTRIES: usize = 4096;
const MAX_APPROVE_ATTEMPTS: usize = 128;

/// Error from pairing store operations.
#[derive(Debug, thiserror::Error)]
pub enum PairingStoreError {
    #[error("Invalid channel: {0}")]
    InvalidChannel(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid pairing data: {0}")]
    InvalidData(String),

    #[error("Rate limit: too many failed approve attempts; try again later")]
    ApproveRateLimited,
}

/// Result of upserting a pairing request.
#[derive(Debug)]
pub struct UpsertResult {
    pub code: String,
    pub created: bool,
}

/// A pending pairing request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub id: String,
    pub code: String,
    pub created_at: String,
    pub last_seen_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PairingStoreFile {
    version: u8,
    requests: Vec<PairingRequest>,
}

#[derive(Debug, Serialize, Deserialize)]
struct AllowFromStoreFile {
    version: u8,
    #[serde(rename = "allowFrom")]
    allow_from: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BlockFromStoreFile {
    version: u8,
    #[serde(rename = "blockFrom")]
    block_from: Vec<String>,
}

fn default_pairing_dir() -> PathBuf {
    thinclaw_platform::resolve_thinclaw_home()
}

fn safe_channel_key(channel: &str) -> Result<String, PairingStoreError> {
    let raw = channel.trim().to_lowercase();
    if !(1..=64).contains(&raw.len())
        || !raw
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !raw.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
    {
        return Err(PairingStoreError::InvalidChannel(channel.to_string()));
    }
    Ok(raw)
}

fn pairing_path(base_dir: &Path, channel: &str) -> Result<PathBuf, PairingStoreError> {
    let key = safe_channel_key(channel)?;
    Ok(base_dir.join(format!("{}-pairing.json", key)))
}

fn allow_from_path(base_dir: &Path, channel: &str) -> Result<PathBuf, PairingStoreError> {
    let key = safe_channel_key(channel)?;
    Ok(base_dir.join(format!("{}-allowFrom.json", key)))
}

fn approve_attempts_path(base_dir: &Path, channel: &str) -> Result<PathBuf, PairingStoreError> {
    let key = safe_channel_key(channel)?;
    Ok(base_dir.join(format!("{}-approve-attempts.json", key)))
}

fn block_from_path(base_dir: &Path, channel: &str) -> Result<PathBuf, PairingStoreError> {
    let key = safe_channel_key(channel)?;
    Ok(base_dir.join(format!("{}-blockFrom.json", key)))
}

fn channel_lock_path(base_dir: &Path, channel: &str) -> Result<PathBuf, PairingStoreError> {
    let key = safe_channel_key(channel)?;
    Ok(base_dir.join(format!("{key}-pairing.lock")))
}

fn validate_identity(value: &str) -> Result<String, PairingStoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_PAIRING_ID_BYTES
        || trimmed.chars().any(char::is_control)
    {
        return Err(PairingStoreError::InvalidData(
            "identity is empty, oversized, or contains control characters".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn read_bounded_file(path: &Path) -> Result<Option<Vec<u8>>, PairingStoreError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PairingStoreError::InvalidData(
            "pairing path is not a regular file".to_string(),
        ));
    }
    if metadata.len() > MAX_PAIRING_FILE_BYTES as u64 {
        return Err(PairingStoreError::InvalidData(
            "pairing file exceeds the size limit".to_string(),
        ));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.len() > MAX_PAIRING_FILE_BYTES as u64 {
        return Err(PairingStoreError::InvalidData(
            "opened pairing file is invalid or oversized".to_string(),
        ));
    }
    let mut bytes = Vec::with_capacity(
        usize::try_from(opened.len())
            .unwrap_or(MAX_PAIRING_FILE_BYTES)
            .min(MAX_PAIRING_FILE_BYTES),
    );
    std::io::Read::by_ref(&mut file)
        .take((MAX_PAIRING_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_PAIRING_FILE_BYTES {
        return Err(PairingStoreError::InvalidData(
            "pairing file exceeds the size limit".to_string(),
        ));
    }
    Ok(Some(bytes))
}

fn atomic_write_file(path: &Path, bytes: &[u8]) -> Result<(), PairingStoreError> {
    if bytes.len() > MAX_PAIRING_FILE_BYTES {
        return Err(PairingStoreError::InvalidData(
            "serialized pairing file exceeds the size limit".to_string(),
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        PairingStoreError::InvalidData("pairing path has no parent directory".to_string())
    })?;
    fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            PairingStoreError::InvalidData("pairing path has no valid filename".to_string())
        })?;
    let tmp_path = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = options.open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp_path, path)?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result.map_err(PairingStoreError::from)
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ApproveAttemptsFile {
    failed_at: Vec<u64>,
}

fn now_iso() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    #[allow(clippy::cast_possible_wrap)]
    chrono::DateTime::from_timestamp(now.as_secs() as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| now.as_secs().to_string())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_timestamp(value: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.timestamp() as u64)
        .or_else(|| value.parse::<u64>().ok())
}

fn is_expired(req: &PairingRequest, now_secs: u64) -> bool {
    let created = parse_timestamp(&req.created_at).unwrap_or(0);
    now_secs.saturating_sub(created) > PAIRING_PENDING_TTL_SECS
}

fn random_code() -> String {
    let mut rng = rand::rng();
    (0..PAIRING_CODE_LENGTH)
        .map(|_| {
            let idx = rng.random_range(0..PAIRING_ALPHABET.len());
            PAIRING_ALPHABET[idx] as char
        })
        .collect()
}

fn generate_unique_code(existing: &HashSet<String>) -> String {
    let mut rng = rand::rng();
    for _ in 0..500 {
        let code = random_code();
        if !existing.contains(&code) {
            return code;
        }
    }
    // Fallback: add suffix
    format!("{}{:04}", random_code(), rng.random_range(0..10000))
}

/// Pairing store for a channel.
#[derive(Debug, Clone)]
pub struct PairingStore {
    base_dir: PathBuf,
}

impl PairingStore {
    /// Create a new pairing store using default directory (~/.thinclaw).
    pub fn new() -> Self {
        Self {
            base_dir: default_pairing_dir(),
        }
    }

    /// Create a pairing store with a custom base directory (for testing).
    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn lock_channel(&self, channel: &str) -> Result<fs::File, PairingStoreError> {
        let path = channel_lock_path(&self.base_dir, channel)?;
        let parent = path.parent().ok_or_else(|| {
            PairingStoreError::InvalidData("pairing lock path has no parent".to_string())
        })?;
        fs::create_dir_all(parent)?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(path)?;
        file.lock_exclusive()?;
        Ok(file)
    }

    fn read_pairing_file(&self, channel: &str) -> Result<PairingStoreFile, PairingStoreError> {
        let path = pairing_path(&self.base_dir, channel)?;
        let Some(bytes) = read_bounded_file(&path)? else {
            return Ok(PairingStoreFile {
                version: 1,
                requests: Vec::new(),
            });
        };
        let file: PairingStoreFile = if bytes.iter().all(u8::is_ascii_whitespace) {
            PairingStoreFile {
                version: 1,
                requests: Vec::new(),
            }
        } else {
            serde_json::from_slice(&bytes)?
        };
        if file.version != 1 || file.requests.len() > PAIRING_PENDING_MAX {
            return Err(PairingStoreError::InvalidData(
                "pairing request file has an unsupported version or too many entries".to_string(),
            ));
        }
        for request in &file.requests {
            validate_identity(&request.id)?;
            if request.code.is_empty()
                || request.code.len() > 16
                || !request
                    .code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric())
                || request.created_at.len() > 64
                || request.last_seen_at.len() > 64
                || parse_timestamp(&request.created_at).is_none()
                || parse_timestamp(&request.last_seen_at).is_none()
                || request.meta.as_ref().is_some_and(|meta| {
                    serde_json::to_vec(meta)
                        .map(|bytes| bytes.len() > MAX_PAIRING_META_BYTES)
                        .unwrap_or(true)
                })
            {
                return Err(PairingStoreError::InvalidData(
                    "pairing request contains malformed or oversized data".to_string(),
                ));
            }
        }
        Ok(file)
    }

    fn write_pairing_requests(
        &self,
        channel: &str,
        requests: &[PairingRequest],
    ) -> Result<(), PairingStoreError> {
        if requests.len() > PAIRING_PENDING_MAX {
            return Err(PairingStoreError::InvalidData(
                "too many pending pairing requests".to_string(),
            ));
        }
        let bytes = serde_json::to_vec_pretty(&PairingStoreFile {
            version: 1,
            requests: requests.to_vec(),
        })?;
        atomic_write_file(&pairing_path(&self.base_dir, channel)?, &bytes)
    }

    fn read_access_entries(
        &self,
        path: &Path,
        allow: bool,
    ) -> Result<Vec<String>, PairingStoreError> {
        let Some(bytes) = read_bounded_file(path)? else {
            return Ok(Vec::new());
        };
        let entries = if allow {
            let file: AllowFromStoreFile = serde_json::from_slice(&bytes)?;
            if file.version != 1 {
                return Err(PairingStoreError::InvalidData(
                    "unsupported allow-list version".to_string(),
                ));
            }
            file.allow_from
        } else {
            let file: BlockFromStoreFile = serde_json::from_slice(&bytes)?;
            if file.version != 1 {
                return Err(PairingStoreError::InvalidData(
                    "unsupported block-list version".to_string(),
                ));
            }
            file.block_from
        };
        if entries.len() > MAX_ACCESS_ENTRIES {
            return Err(PairingStoreError::InvalidData(
                "access list exceeds the entry limit".to_string(),
            ));
        }
        for entry in &entries {
            validate_identity(entry)?;
        }
        Ok(entries)
    }

    fn write_access_entries(
        &self,
        path: &Path,
        entries: &[String],
        allow: bool,
    ) -> Result<(), PairingStoreError> {
        if entries.len() > MAX_ACCESS_ENTRIES {
            return Err(PairingStoreError::InvalidData(
                "access list exceeds the entry limit".to_string(),
            ));
        }
        for entry in entries {
            validate_identity(entry)?;
        }
        let bytes = if allow {
            serde_json::to_vec_pretty(&AllowFromStoreFile {
                version: 1,
                allow_from: entries.to_vec(),
            })?
        } else {
            serde_json::to_vec_pretty(&BlockFromStoreFile {
                version: 1,
                block_from: entries.to_vec(),
            })?
        };
        atomic_write_file(path, &bytes)
    }

    fn read_approve_attempts(
        &self,
        channel: &str,
    ) -> Result<ApproveAttemptsFile, PairingStoreError> {
        let path = approve_attempts_path(&self.base_dir, channel)?;
        let Some(bytes) = read_bounded_file(&path)? else {
            return Ok(ApproveAttemptsFile::default());
        };
        let data: ApproveAttemptsFile = serde_json::from_slice(&bytes)?;
        if data.failed_at.len() > MAX_APPROVE_ATTEMPTS {
            return Err(PairingStoreError::InvalidData(
                "approval-attempt file exceeds the entry limit".to_string(),
            ));
        }
        Ok(data)
    }

    fn write_approve_attempts(
        &self,
        channel: &str,
        data: &ApproveAttemptsFile,
    ) -> Result<(), PairingStoreError> {
        if data.failed_at.len() > MAX_APPROVE_ATTEMPTS {
            return Err(PairingStoreError::InvalidData(
                "approval-attempt file exceeds the entry limit".to_string(),
            ));
        }
        atomic_write_file(
            &approve_attempts_path(&self.base_dir, channel)?,
            &serde_json::to_vec_pretty(data)?,
        )
    }

    /// List pending pairing requests for a channel.
    pub fn list_pending(&self, channel: &str) -> Result<Vec<PairingRequest>, PairingStoreError> {
        let _lock = self.lock_channel(channel)?;
        let file = self.read_pairing_file(channel)?;

        let now = now_secs();
        let original_len = file.requests.len();
        let mut requests: Vec<_> = file
            .requests
            .into_iter()
            .filter(|r| !is_expired(r, now))
            .collect();

        if requests.len() != original_len {
            self.write_pairing_requests(channel, &requests)?;
        }

        requests.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(requests)
    }

    /// Upsert a pairing request. Returns (code, created).
    pub fn upsert_request(
        &self,
        channel: &str,
        id: &str,
        meta: Option<serde_json::Value>,
    ) -> Result<UpsertResult, PairingStoreError> {
        let id = validate_identity(id)?;
        if meta.as_ref().is_some_and(|value| {
            serde_json::to_vec(value)
                .map(|bytes| bytes.len() > MAX_PAIRING_META_BYTES)
                .unwrap_or(true)
        }) {
            return Err(PairingStoreError::InvalidData(
                "pairing metadata exceeds the size limit".to_string(),
            ));
        }
        let _lock = self.lock_channel(channel)?;
        let mut store = self.read_pairing_file(channel)?;

        let now = now_iso();
        let now_secs = now_secs();

        store.requests.retain(|r| !is_expired(r, now_secs));
        let existing_codes: HashSet<String> = store
            .requests
            .iter()
            .map(|r| r.code.to_uppercase())
            .collect();

        if let Some(idx) = store.requests.iter().position(|r| r.id == id) {
            let req = &mut store.requests[idx];
            let code = if req.code.is_empty() {
                generate_unique_code(&existing_codes)
            } else {
                req.code.clone()
            };
            req.last_seen_at = now.clone();
            req.code = code.clone();
            if let Some(m) = meta {
                req.meta = Some(m);
            }
            self.write_pairing_requests(channel, &store.requests)?;
            return Ok(UpsertResult {
                code,
                created: false,
            });
        }

        if store.requests.len() >= PAIRING_PENDING_MAX {
            return Ok(UpsertResult {
                code: String::new(),
                created: false,
            });
        }

        let code = generate_unique_code(&existing_codes);
        store.requests.push(PairingRequest {
            id: id.clone(),
            code: code.clone(),
            created_at: now.clone(),
            last_seen_at: now,
            meta,
        });

        self.write_pairing_requests(channel, &store.requests)?;

        Ok(UpsertResult {
            code,
            created: true,
        })
    }

    fn is_approve_rate_limited(&self, channel: &str) -> Result<bool, PairingStoreError> {
        let mut data = self.read_approve_attempts(channel)?;
        let now = now_secs();
        let cutoff = now.saturating_sub(PAIRING_APPROVE_RATE_WINDOW_SECS);
        let original_len = data.failed_at.len();
        data.failed_at.retain(|&t| t >= cutoff);
        if data.failed_at.len() != original_len {
            self.write_approve_attempts(channel, &data)?;
        }
        Ok(data.failed_at.len() >= PAIRING_APPROVE_RATE_LIMIT)
    }

    fn record_failed_approve(&self, channel: &str) -> Result<(), PairingStoreError> {
        let mut data = self.read_approve_attempts(channel)?;

        let now = now_secs();
        data.failed_at.push(now);
        let cutoff = now.saturating_sub(PAIRING_APPROVE_RATE_WINDOW_SECS);
        data.failed_at.retain(|&t| t >= cutoff);

        self.write_approve_attempts(channel, &data)
    }

    /// Approve a pairing code and add the sender to allowFrom.
    pub fn approve(
        &self,
        channel: &str,
        code: &str,
    ) -> Result<Option<PairingRequest>, PairingStoreError> {
        let code = code.trim().to_uppercase();
        if code.is_empty() {
            return Ok(None);
        }
        if code.len() > 16 || !code.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Ok(None);
        }

        let _lock = self.lock_channel(channel)?;

        if self.is_approve_rate_limited(channel)? {
            return Err(PairingStoreError::ApproveRateLimited);
        }

        let path = pairing_path(&self.base_dir, channel)?;
        if read_bounded_file(&path)?.is_none() {
            return Err(PairingStoreError::InvalidChannel(
                "no pairing file".to_string(),
            ));
        }
        let mut store = self.read_pairing_file(channel)?;

        let now_secs = now_secs();
        let before_expiry = store.requests.len();
        store.requests.retain(|r| !is_expired(r, now_secs));

        let idx = store.requests.iter().position(|request| {
            use subtle::ConstantTimeEq as _;
            request.code.len() == code.len()
                && bool::from(request.code.as_bytes().ct_eq(code.as_bytes()))
        });

        let entry = match idx {
            Some(i) => store.requests.remove(i),
            None => {
                if store.requests.len() != before_expiry {
                    self.write_pairing_requests(channel, &store.requests)?;
                }
                self.record_failed_approve(channel)?;
                return Ok(None);
            }
        };

        // Publish authorization first. If the subsequent pending-file write
        // fails, the caller is still safely authorized and can retry cleanup;
        // the inverse order could lose a valid approval entirely.
        self.add_allow_from_unlocked(channel, &entry.id)?;
        self.write_pairing_requests(channel, &store.requests)?;

        Ok(Some(entry))
    }

    /// Find a pending pairing request by approval code without mutating state.
    pub fn find_pending_by_code(
        &self,
        channel: &str,
        code: &str,
    ) -> Result<Option<PairingRequest>, PairingStoreError> {
        let code = code.trim().to_uppercase();
        if code.is_empty() {
            return Ok(None);
        }

        let requests = self.list_pending(channel)?;
        Ok(requests
            .into_iter()
            .find(|request| request.code.to_uppercase() == code))
    }

    /// Restore a pending pairing request, preserving its original code and metadata.
    pub fn restore_pending_request(
        &self,
        channel: &str,
        request: &PairingRequest,
    ) -> Result<(), PairingStoreError> {
        validate_identity(&request.id)?;
        if request.code.is_empty()
            || request.code.len() > 16
            || !request
                .code
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
            || request.meta.as_ref().is_some_and(|value| {
                serde_json::to_vec(value)
                    .map(|bytes| bytes.len() > MAX_PAIRING_META_BYTES)
                    .unwrap_or(true)
            })
        {
            return Err(PairingStoreError::InvalidData(
                "restored pairing request is malformed or oversized".to_string(),
            ));
        }
        let _lock = self.lock_channel(channel)?;
        let mut store = self.read_pairing_file(channel)?;

        store.requests.retain(|existing| existing.id != request.id);
        if store.requests.len() >= PAIRING_PENDING_MAX {
            return Err(PairingStoreError::InvalidData(
                "too many pending pairing requests".to_string(),
            ));
        }
        store.requests.push(request.clone());

        self.write_pairing_requests(channel, &store.requests)
    }

    /// Read the allowFrom list for a channel.
    pub fn read_allow_from(&self, channel: &str) -> Result<Vec<String>, PairingStoreError> {
        let path = allow_from_path(&self.base_dir, channel)?;
        self.read_access_entries(&path, true)
    }

    /// Ensure an entry exists in the allowFrom list without requiring a
    /// pending pairing request first.
    pub fn ensure_allow_from(&self, channel: &str, entry: &str) -> Result<(), PairingStoreError> {
        let entry = validate_identity(entry)?;
        let _lock = self.lock_channel(channel)?;
        self.add_allow_from_unlocked(channel, &entry)
    }

    /// Check if a sender is allowed (by id or username).
    ///
    /// Returns `false` if the sender is on the block list, even if they
    /// appear in the allow list (blocklist takes precedence).
    pub fn is_sender_allowed(
        &self,
        channel: &str,
        id: &str,
        username: Option<&str>,
    ) -> Result<bool, PairingStoreError> {
        // Blocklist takes precedence
        if self.is_sender_blocked(channel, id, username)? {
            return Ok(false);
        }
        let Ok(id) = validate_identity(id) else {
            return Ok(false);
        };
        let allow = self.read_allow_from(channel)?;
        let id_ok = allow.iter().any(|e| e.trim() == id);
        if id_ok {
            return Ok(true);
        }
        if let Some(u) = username {
            let Ok(u) = validate_identity(u) else {
                return Ok(false);
            };
            let u = u.trim().to_lowercase();
            let u_norm = u.strip_prefix('@').unwrap_or(&u);
            if allow.iter().any(|e| {
                e.trim().to_lowercase() == u || e.trim().to_lowercase() == format!("@{}", u_norm)
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // Block list

    /// Read the blockFrom list for a channel.
    pub fn read_block_from(&self, channel: &str) -> Result<Vec<String>, PairingStoreError> {
        let path = block_from_path(&self.base_dir, channel)?;
        self.read_access_entries(&path, false)
    }

    /// Add an entry to the blockFrom list for a channel.
    pub fn add_block_from(&self, channel: &str, entry: &str) -> Result<(), PairingStoreError> {
        let entry = validate_identity(entry)?;
        let _lock = self.lock_channel(channel)?;
        self.add_access_entry_unlocked(channel, &entry, false)
    }

    /// Remove an entry from the blockFrom list for a channel.
    pub fn remove_block_from(&self, channel: &str, entry: &str) -> Result<bool, PairingStoreError> {
        let Ok(entry) = validate_identity(entry) else {
            return Ok(false);
        };
        let _lock = self.lock_channel(channel)?;
        self.remove_access_entry_unlocked(channel, &entry, false)
    }

    /// Check if a sender is on the block list (by id or username).
    pub fn is_sender_blocked(
        &self,
        channel: &str,
        id: &str,
        username: Option<&str>,
    ) -> Result<bool, PairingStoreError> {
        let blocked = self.read_block_from(channel)?;
        if blocked.is_empty() {
            return Ok(false);
        }
        let Ok(id) = validate_identity(id) else {
            return Ok(false);
        };
        if blocked.iter().any(|e| e.trim() == id) {
            return Ok(true);
        }
        if let Some(u) = username {
            let Ok(u) = validate_identity(u) else {
                return Ok(false);
            };
            let u = u.trim().to_lowercase();
            let u_norm = u.strip_prefix('@').unwrap_or(&u);
            if blocked.iter().any(|e| {
                e.trim().to_lowercase() == u || e.trim().to_lowercase() == format!("@{}", u_norm)
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn add_allow_from_unlocked(&self, channel: &str, entry: &str) -> Result<(), PairingStoreError> {
        self.add_access_entry_unlocked(channel, entry, true)
    }

    pub fn remove_allow_from(&self, channel: &str, entry: &str) -> Result<bool, PairingStoreError> {
        let Ok(entry) = validate_identity(entry) else {
            return Ok(false);
        };
        let _lock = self.lock_channel(channel)?;
        self.remove_access_entry_unlocked(channel, &entry, true)
    }

    fn add_access_entry_unlocked(
        &self,
        channel: &str,
        entry: &str,
        allow: bool,
    ) -> Result<(), PairingStoreError> {
        let entry = validate_identity(entry)?;
        let path = if allow {
            allow_from_path(&self.base_dir, channel)?
        } else {
            block_from_path(&self.base_dir, channel)?
        };
        let mut entries = self.read_access_entries(&path, allow)?;
        let normalized = entry.to_lowercase();
        if entries
            .iter()
            .any(|existing| existing.to_lowercase() == normalized)
        {
            return Ok(());
        }
        if entries.len() >= MAX_ACCESS_ENTRIES {
            return Err(PairingStoreError::InvalidData(
                "access list exceeds the entry limit".to_string(),
            ));
        }
        entries.push(entry);
        self.write_access_entries(&path, &entries, allow)
    }

    fn remove_access_entry_unlocked(
        &self,
        channel: &str,
        entry: &str,
        allow: bool,
    ) -> Result<bool, PairingStoreError> {
        let path = if allow {
            allow_from_path(&self.base_dir, channel)?
        } else {
            block_from_path(&self.base_dir, channel)?
        };
        let mut entries = self.read_access_entries(&path, allow)?;
        let normalized = entry.trim().to_lowercase();
        let original_len = entries.len();
        entries.retain(|value| value.trim().to_lowercase() != normalized);
        if entries.len() == original_len {
            return Ok(false);
        }
        self.write_access_entries(&path, &entries, allow)?;
        Ok(true)
    }
}

impl Default for PairingStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_safe_channel_key() {
        assert_eq!(safe_channel_key("telegram").unwrap(), "telegram");
        assert_eq!(safe_channel_key("Telegram").unwrap(), "telegram");
        safe_channel_key("").unwrap_err();
    }

    #[test]
    fn test_random_code() {
        let c = random_code();
        assert_eq!(c.len(), PAIRING_CODE_LENGTH);
        assert!(c.chars().all(|c| PAIRING_ALPHABET.contains(&(c as u8))));
    }

    fn test_store() -> (PairingStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = PairingStore::with_base_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn test_list_pending_empty() {
        let (store, _) = test_store();
        let requests = store.list_pending("telegram").unwrap();
        assert!(requests.is_empty());
    }

    #[test]
    fn test_upsert_request_creates_new() {
        let (store, _) = test_store();
        let result = store
            .upsert_request(
                "telegram",
                "user123",
                Some(serde_json::json!({"chat_id": 456})),
            )
            .unwrap();
        assert!(result.created);
        assert_eq!(result.code.len(), PAIRING_CODE_LENGTH);
        assert!(
            result
                .code
                .chars()
                .all(|c| PAIRING_ALPHABET.contains(&(c as u8)))
        );
    }

    #[test]
    fn test_upsert_request_updates_existing() {
        let (store, _) = test_store();
        let r1 = store.upsert_request("telegram", "user123", None).unwrap();
        assert!(r1.created);
        let r2 = store
            .upsert_request("telegram", "user123", Some(serde_json::json!({"x": 1})))
            .unwrap();
        assert!(!r2.created);
        assert_eq!(r1.code, r2.code);

        let pending = store.list_pending("telegram").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "user123");
        assert_eq!(pending[0].meta, Some(serde_json::json!({"x": 1})));
    }

    #[test]
    fn test_approve_adds_to_allow_from() {
        let (store, _) = test_store();
        let r = store.upsert_request("telegram", "user456", None).unwrap();
        assert!(r.created);

        let approved = store.approve("telegram", &r.code).unwrap();
        assert!(approved.is_some());
        assert_eq!(approved.unwrap().id, "user456");

        let allow = store.read_allow_from("telegram").unwrap();
        assert_eq!(allow, vec!["user456"]);
    }

    #[test]
    fn test_ensure_allow_from_adds_and_deduplicates() {
        let (store, _) = test_store();
        store.ensure_allow_from("telegram", "owner123").unwrap();
        store.ensure_allow_from("telegram", "OWNER123").unwrap();

        let allow = store.read_allow_from("telegram").unwrap();
        assert_eq!(allow, vec!["owner123"]);
    }

    #[test]
    fn test_approve_case_insensitive_code() {
        let (store, _) = test_store();
        let r = store.upsert_request("telegram", "user789", None).unwrap();
        let code_lower = r.code.to_lowercase();
        let approved = store.approve("telegram", &code_lower).unwrap();
        assert!(approved.is_some());
    }

    #[test]
    fn test_approve_invalid_code_returns_none() {
        let (store, _) = test_store();
        store.upsert_request("telegram", "user123", None).unwrap();
        let approved = store.approve("telegram", "BADCODE1").unwrap();
        assert!(approved.is_none());
    }

    #[test]
    fn test_approve_rate_limited_after_many_failures() {
        let (store, _) = test_store();
        store.upsert_request("telegram", "user123", None).unwrap();
        for _ in 0..PAIRING_APPROVE_RATE_LIMIT {
            let _ = store.approve("telegram", "WRONG01");
        }
        let err = store.approve("telegram", "WRONG02").unwrap_err();
        assert!(matches!(err, PairingStoreError::ApproveRateLimited));
    }

    #[test]
    fn test_is_sender_allowed_by_id() {
        let (store, _) = test_store();
        let r = store.upsert_request("telegram", "user999", None).unwrap();
        store.approve("telegram", &r.code).unwrap();

        assert!(
            store
                .is_sender_allowed("telegram", "user999", None)
                .unwrap()
        );
        assert!(!store.is_sender_allowed("telegram", "other", None).unwrap());
    }

    #[test]
    fn test_is_sender_allowed_by_username() {
        let (store, _) = test_store();
        store
            .upsert_request(
                "telegram",
                "alice",
                Some(serde_json::json!({"username": "alice"})),
            )
            .unwrap();
        let pending = store.list_pending("telegram").unwrap();
        store.approve("telegram", &pending[0].code).unwrap();

        // approve adds id to allow_from. For username we need to add it manually.
        // Actually approve adds entry.id which is "alice". So is_sender_allowed("telegram", "alice", None) would work.
        assert!(store.is_sender_allowed("telegram", "alice", None).unwrap());
        assert!(
            store
                .is_sender_allowed("telegram", "alice", Some("alice"))
                .unwrap()
        );
    }

    #[test]
    fn test_channel_normalization() {
        let (store, _) = test_store();
        store.upsert_request("Telegram", "u1", None).unwrap();
        let pending = store.list_pending("telegram").unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "u1");
    }

    #[test]
    fn test_invalid_channel_rejected() {
        let (store, _) = test_store();
        store.upsert_request("telegram", "u1", None).unwrap();
        store.list_pending("").unwrap_err();
        store.upsert_request("", "u1", None).unwrap_err();
    }

    // Block list tests

    #[test]
    fn test_block_from_empty_by_default() {
        let (store, _) = test_store();
        let blocked = store.read_block_from("telegram").unwrap();
        assert!(blocked.is_empty());
    }

    #[test]
    fn test_add_and_read_block_from() {
        let (store, _) = test_store();
        store.add_block_from("telegram", "spammer123").unwrap();
        store.add_block_from("telegram", "baduser456").unwrap();
        let blocked = store.read_block_from("telegram").unwrap();
        assert_eq!(blocked.len(), 2);
        assert!(blocked.contains(&"spammer123".to_string()));
        assert!(blocked.contains(&"baduser456".to_string()));
    }

    #[test]
    fn test_add_block_from_deduplicates() {
        let (store, _) = test_store();
        store.add_block_from("telegram", "spammer123").unwrap();
        store.add_block_from("telegram", "SPAMMER123").unwrap(); // case-insensitive dupe
        let blocked = store.read_block_from("telegram").unwrap();
        assert_eq!(blocked.len(), 1);
    }

    #[test]
    fn test_remove_block_from() {
        let (store, _) = test_store();
        store.add_block_from("telegram", "spammer123").unwrap();
        store.add_block_from("telegram", "other").unwrap();

        let removed = store.remove_block_from("telegram", "spammer123").unwrap();
        assert!(removed);
        let blocked = store.read_block_from("telegram").unwrap();
        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0], "other");

        let not_found = store.remove_block_from("telegram", "nonexistent").unwrap();
        assert!(!not_found);
    }

    #[test]
    fn test_remove_block_from_no_file() {
        let (store, _) = test_store();
        // No block file exists yet - should return false, not error.
        let removed = store.remove_block_from("telegram", "nobody").unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_blocklist_takes_precedence_over_allowlist() {
        let (store, _) = test_store();
        // Add user to allowlist via approve
        let r = store.upsert_request("telegram", "user123", None).unwrap();
        store.approve("telegram", &r.code).unwrap();
        // Confirm they're allowed
        assert!(
            store
                .is_sender_allowed("telegram", "user123", None)
                .unwrap()
        );

        // Now block them
        store.add_block_from("telegram", "user123").unwrap();
        // Blocklist takes precedence - should NOT be allowed
        assert!(
            !store
                .is_sender_allowed("telegram", "user123", None)
                .unwrap()
        );
        // And explicitly blocked
        assert!(
            store
                .is_sender_blocked("telegram", "user123", None)
                .unwrap()
        );
    }

    #[test]
    fn test_is_sender_blocked_by_username() {
        let (store, _) = test_store();
        store.add_block_from("telegram", "@badbot").unwrap();
        assert!(
            store
                .is_sender_blocked("telegram", "other_id", Some("badbot"))
                .unwrap()
        );
        assert!(
            store
                .is_sender_blocked("telegram", "other_id", Some("@badbot"))
                .unwrap()
        );
        assert!(
            !store
                .is_sender_blocked("telegram", "other_id", Some("goodbot"))
                .unwrap()
        );
    }

    #[test]
    fn test_is_sender_blocked_empty_list() {
        let (store, _) = test_store();
        assert!(!store.is_sender_blocked("telegram", "anyone", None).unwrap());
    }
}
