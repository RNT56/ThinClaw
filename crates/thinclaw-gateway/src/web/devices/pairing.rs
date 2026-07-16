//! Pending device-pairing store: `~/.thinclaw/device-pairing.json`.
//!
//! Mirrors `crates/thinclaw-channels/src/pairing.rs` mechanics (fs4 locking,
//! versioned JSON, tmp+rename atomic writes, `with_base_dir` for tests) but
//! authorizes *API clients* rather than chat senders (D-P4) — hence a
//! separate store file.
//!
//! ## Secret / code storage
//!
//! Each pending record stores only `sha256(secret)` and `sha256(code)` —
//! never the raw values. The raw 32-byte secret and 8-char human code are
//! handed to the caller once, at `create_pairing` time, and never persisted.
//!
//! ## `require_confirm` flow
//!
//! In the default (`require_confirm = false`) mode, `consume` is a single
//! atomic step: presenting the correct secret/code immediately finalizes
//! the pairing (D-P3 — possession of the one-time secret is proof the
//! operator initiated pairing from an already-authenticated surface).
//!
//! When `device_pairing.require_confirm = true`, `consume` instead becomes
//! two steps:
//!
//! 1. First `consume(secret_or_code)`: the credential matches, but instead
//!    of returning `Consumed`, the pending record is marked
//!    `awaiting_confirm = true` (the secret/code hash is retained, *not*
//!    deleted) and `ConsumeOutcome::AwaitingConfirm` is returned.
//! 2. An operator-facing `approve(pairing_id)` call marks the record
//!    `approved = true`.
//! 3. A repeat `consume(secret_or_code)` with the *same* credential now
//!    finalizes the pairing (`ConsumeOutcome::Consumed`) and removes the
//!    pending record (single-use).
//!
//! A record can only be `awaiting_confirm` for one secret at a time; the
//! record is removed on final consume either way, so the credential remains
//! single-use across both steps.

use std::fs;
use std::io::{Read as _, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::FileExt;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

const PENDING_FILE_NAME: &str = "device-pairing.json";
const ATTEMPTS_FILE_NAME: &str = "device-pairing-attempts.json";
const FILE_VERSION: u8 = 1;

/// TTL for pending device-pairing requests (D-P1 / mirrors pairing.rs).
pub const PAIRING_PENDING_TTL_SECS: u64 = 15 * 60;
/// Max outstanding pending pairing requests at once.
pub const PAIRING_PENDING_MAX: usize = 3;
/// Max failed consume attempts per window before lockout.
pub const PAIRING_FAILED_LIMIT: usize = 10;
/// Lockout window (seconds).
pub const PAIRING_FAILED_WINDOW_SECS: u64 = 5 * 60;

const HUMAN_CODE_LENGTH: usize = 8;
/// Same alphabet as `crates/thinclaw-channels/src/pairing.rs` (avoids
/// visually ambiguous characters: no `0/O`, `1/I/L`).
const HUMAN_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Debug, thiserror::Error)]
pub enum DevicePairingError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("too many pending pairing requests; try again later")]
    TooManyPending,

    #[error("rate limit: too many failed pairing attempts; try again later")]
    RateLimited,
}

/// A pending device-pairing record, as persisted on disk. Secret/code are
/// stored only as SHA-256 hashes.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PendingRecord {
    pairing_id: String,
    name: String,
    secret_hash: String,
    code_hash: String,
    created_at: u64,
    expires_at: u64,
    /// Set by the first `consume` call under `require_confirm` mode.
    #[serde(default)]
    awaiting_confirm: bool,
    /// Set by the admin `approve` call.
    #[serde(default)]
    approved: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingFile {
    version: u8,
    pending: Vec<PendingRecord>,
}

impl Default for PendingFile {
    fn default() -> Self {
        Self {
            version: FILE_VERSION,
            pending: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct AttemptsFile {
    failed_at: Vec<u64>,
}

/// Result of `create_pairing`: the raw, one-time-visible secret and code
/// plus the record id needed for the `require_confirm` approve step.
#[derive(Debug)]
pub struct CreatedPairing {
    pub pairing_id: String,
    /// Raw base64url 32-byte secret. Never persisted.
    pub secret: String,
    /// Raw 8-char human code. Never persisted.
    pub code: String,
    pub created_at: u64,
    pub expires_at: u64,
}

/// Outcome of a `consume` call.
#[derive(Debug, PartialEq, Eq)]
pub enum ConsumeOutcome {
    /// Credential matched and the pairing is fully consumed (single-use;
    /// record removed).
    Consumed { pairing_id: String, name: String },
    /// `require_confirm` mode: credential matched but the record is now
    /// waiting on an admin `approve(pairing_id)` call before a repeat
    /// `consume` finalizes it.
    AwaitingConfirm { pairing_id: String },
    /// No pending record matched the credential (or it was expired).
    NotFound,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sha256_hex(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

fn base64_url_no_pad(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    base64_url_no_pad(&bytes)
}

fn random_human_code() -> String {
    let mut rng = rand::rng();
    (0..HUMAN_CODE_LENGTH)
        .map(|_| {
            let idx = rng.random_range(0..HUMAN_CODE_ALPHABET.len());
            HUMAN_CODE_ALPHABET[idx] as char
        })
        .collect()
}

fn is_expired(record: &PendingRecord, now: u64) -> bool {
    now > record.expires_at
}

fn parse_json_or_default<T>(content: &str) -> Result<T, DevicePairingError>
where
    T: serde::de::DeserializeOwned + Default,
{
    if content.trim().is_empty() {
        Ok(T::default())
    } else {
        serde_json::from_str(content).map_err(DevicePairingError::from)
    }
}

/// Store of pending device-pairing attempts.
#[derive(Debug, Clone)]
pub struct DevicePairingStore {
    base_dir: PathBuf,
    require_confirm: bool,
}

impl DevicePairingStore {
    pub fn new() -> Self {
        Self {
            base_dir: thinclaw_platform::resolve_thinclaw_home(),
            require_confirm: false,
        }
    }

    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            require_confirm: false,
        }
    }

    /// Opt into the `require_confirm` two-step flow (mirrors
    /// `device_pairing.require_confirm` setting).
    pub fn with_require_confirm(mut self, require_confirm: bool) -> Self {
        self.require_confirm = require_confirm;
        self
    }

    fn pending_path(&self) -> PathBuf {
        self.base_dir.join(PENDING_FILE_NAME)
    }

    fn attempts_path(&self) -> PathBuf {
        self.base_dir.join(ATTEMPTS_FILE_NAME)
    }

    fn open_locked(path: &Path) -> Result<fs::File, DevicePairingError> {
        fs::create_dir_all(path.parent().expect("path always has a parent"))?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        FileExt::lock(&file)?;
        Ok(file)
    }

    fn read_pending_locked(file: &mut fs::File) -> Result<PendingFile, DevicePairingError> {
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        parse_json_or_default(&content)
    }

    fn write_pending_locked(
        file: &mut fs::File,
        path: &Path,
        data: &PendingFile,
    ) -> Result<(), DevicePairingError> {
        let json = serde_json::to_string_pretty(data)?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, &json)?;
        fs::rename(&tmp_path, path)?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        Ok(())
    }

    fn purge_expired(data: &mut PendingFile) {
        let now = now_secs();
        data.pending.retain(|r| !is_expired(r, now));
    }

    /// Create a new pending pairing request. Purges expired entries first;
    /// enforces `PAIRING_PENDING_MAX`.
    pub fn create_pairing(&self, name: &str) -> Result<CreatedPairing, DevicePairingError> {
        let path = self.pending_path();
        let mut file = Self::open_locked(&path)?;
        let mut data = Self::read_pending_locked(&mut file)?;
        Self::purge_expired(&mut data);

        if data.pending.len() >= PAIRING_PENDING_MAX {
            FileExt::unlock(&file)?;
            return Err(DevicePairingError::TooManyPending);
        }

        let secret = random_secret();
        let code = random_human_code();
        let created_at = now_secs();
        let expires_at = created_at + PAIRING_PENDING_TTL_SECS;
        let pairing_id = Uuid::new_v4().to_string();

        data.pending.push(PendingRecord {
            pairing_id: pairing_id.clone(),
            name: name.to_string(),
            secret_hash: sha256_hex(&secret),
            code_hash: sha256_hex(&code),
            created_at,
            expires_at,
            awaiting_confirm: false,
            approved: false,
        });

        Self::write_pending_locked(&mut file, &path, &data)?;
        FileExt::unlock(&file)?;

        Ok(CreatedPairing {
            pairing_id,
            secret,
            code,
            created_at,
            expires_at,
        })
    }

    /// List pending records (public/admin view; never exposes hashes).
    pub fn list_pending(&self) -> Result<Vec<PendingPairView>, DevicePairingError> {
        let path = self.pending_path();
        let mut file = Self::open_locked(&path)?;
        let mut data = Self::read_pending_locked(&mut file)?;
        Self::purge_expired(&mut data);
        Self::write_pending_locked(&mut file, &path, &data)?;
        FileExt::unlock(&file)?;

        Ok(data
            .pending
            .into_iter()
            .map(|r| PendingPairView {
                pairing_id: r.pairing_id,
                name: r.name,
                created_at: r.created_at,
                expires_at: r.expires_at,
                awaiting_confirm: r.awaiting_confirm,
            })
            .collect())
    }

    fn is_rate_limited(&self) -> Result<bool, DevicePairingError> {
        let path = self.attempts_path();
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        let mut data: AttemptsFile = parse_json_or_default(&content)?;
        let now = now_secs();
        let cutoff = now.saturating_sub(PAIRING_FAILED_WINDOW_SECS);
        data.failed_at.retain(|&t| t >= cutoff);
        Ok(data.failed_at.len() >= PAIRING_FAILED_LIMIT)
    }

    fn record_failed_attempt(&self) -> Result<(), DevicePairingError> {
        let path = self.attempts_path();
        let mut file = Self::open_locked(&path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;
        let mut data: AttemptsFile = parse_json_or_default(&content)?;

        let now = now_secs();
        data.failed_at.push(now);
        let cutoff = now.saturating_sub(PAIRING_FAILED_WINDOW_SECS);
        data.failed_at.retain(|&t| t >= cutoff);

        let json = serde_json::to_string_pretty(&data)?;
        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, json)?;
        fs::rename(&tmp_path, &path)?;
        FileExt::unlock(&file)?;
        Ok(())
    }

    /// Admin call: mark a pending, `awaiting_confirm` record `approved`.
    /// No-op (returns `false`) if the id doesn't exist or isn't awaiting
    /// confirmation.
    pub fn approve(&self, pairing_id: &str) -> Result<bool, DevicePairingError> {
        let path = self.pending_path();
        let mut file = Self::open_locked(&path)?;
        let mut data = Self::read_pending_locked(&mut file)?;
        Self::purge_expired(&mut data);

        let mut approved = false;
        if let Some(record) = data
            .pending
            .iter_mut()
            .find(|r| r.pairing_id == pairing_id && r.awaiting_confirm)
        {
            record.approved = true;
            approved = true;
        }

        Self::write_pending_locked(&mut file, &path, &data)?;
        FileExt::unlock(&file)?;
        Ok(approved)
    }

    /// Attempt to redeem a secret or human code. Enforces the lockout
    /// before touching pending state; records a failed attempt on no
    /// match. See module docs for the `require_confirm` two-step flow.
    pub fn consume(&self, secret_or_code: &str) -> Result<ConsumeOutcome, DevicePairingError> {
        if self.is_rate_limited()? {
            return Err(DevicePairingError::RateLimited);
        }

        let secret_hash = sha256_hex(secret_or_code);
        // Human codes are case-insensitive (mirrors pairing.rs).
        let code_hash = sha256_hex(&secret_or_code.trim().to_uppercase());

        let path = self.pending_path();
        let mut file = Self::open_locked(&path)?;
        let mut data = Self::read_pending_locked(&mut file)?;
        Self::purge_expired(&mut data);

        // Compare digests with ct_eq: hash-then-compare is already
        // practically timing-safe, but the documented guarantee (and the
        // NETWORK_SECURITY.md review checklist) is constant-time comparison
        // on every secret path, matching registry.rs token validation.
        let idx = data.pending.iter().position(|r| {
            bool::from(r.secret_hash.as_bytes().ct_eq(secret_hash.as_bytes()))
                || bool::from(r.code_hash.as_bytes().ct_eq(code_hash.as_bytes()))
        });

        let Some(idx) = idx else {
            Self::write_pending_locked(&mut file, &path, &data)?;
            FileExt::unlock(&file)?;
            self.record_failed_attempt()?;
            return Ok(ConsumeOutcome::NotFound);
        };

        if self.require_confirm && !data.pending[idx].approved {
            // First touch (or repeat touch pre-approval): mark awaiting
            // confirm and retain the record — do NOT consume yet.
            data.pending[idx].awaiting_confirm = true;
            let pairing_id = data.pending[idx].pairing_id.clone();
            Self::write_pending_locked(&mut file, &path, &data)?;
            FileExt::unlock(&file)?;
            return Ok(ConsumeOutcome::AwaitingConfirm { pairing_id });
        }

        // Either require_confirm is off, or the record has been approved:
        // finalize and remove (single-use).
        let record = data.pending.remove(idx);
        Self::write_pending_locked(&mut file, &path, &data)?;
        FileExt::unlock(&file)?;

        Ok(ConsumeOutcome::Consumed {
            pairing_id: record.pairing_id,
            name: record.name,
        })
    }
}

impl Default for DevicePairingStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Admin-facing view of a pending pairing record (no secret/code material).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPairView {
    pub pairing_id: String,
    pub name: String,
    pub created_at: u64,
    pub expires_at: u64,
    pub awaiting_confirm: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_store() -> (DevicePairingStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = DevicePairingStore::with_base_dir(dir.path().to_path_buf());
        (store, dir)
    }

    #[test]
    fn create_pairing_yields_distinct_secret_and_code() {
        let (store, _dir) = test_store();
        let created = store.create_pairing("Phone").unwrap();
        assert_eq!(created.code.len(), HUMAN_CODE_LENGTH);
        assert!(
            created
                .code
                .chars()
                .all(|c| HUMAN_CODE_ALPHABET.contains(&(c as u8)))
        );
        assert!(!created.secret.is_empty());
        assert_ne!(created.secret, created.code);
    }

    #[test]
    fn raw_secret_and_code_never_persisted_to_disk() {
        let (store, dir) = test_store();
        let created = store.create_pairing("Phone").unwrap();

        let raw = fs::read_to_string(dir.path().join(PENDING_FILE_NAME)).unwrap();
        assert!(!raw.contains(&created.secret));
        assert!(!raw.contains(&created.code));
        assert!(raw.contains(&sha256_hex(&created.secret)));
        assert!(raw.contains(&sha256_hex(&created.code)));
    }

    #[test]
    fn consume_by_secret_is_single_use() {
        let (store, _dir) = test_store();
        let created = store.create_pairing("Phone").unwrap();

        match store.consume(&created.secret).unwrap() {
            ConsumeOutcome::Consumed { name, .. } => assert_eq!(name, "Phone"),
            other => panic!("expected Consumed, got {other:?}"),
        }

        // Second consume of the same secret must fail — single use.
        assert_eq!(
            store.consume(&created.secret).unwrap(),
            ConsumeOutcome::NotFound
        );
    }

    #[test]
    fn consume_by_human_code_case_insensitive() {
        let (store, _dir) = test_store();
        let created = store.create_pairing("Phone").unwrap();
        let lower = created.code.to_lowercase();

        match store.consume(&lower).unwrap() {
            ConsumeOutcome::Consumed { .. } => {}
            other => panic!("expected Consumed, got {other:?}"),
        }
    }

    #[test]
    fn consume_unknown_credential_returns_not_found() {
        let (store, _dir) = test_store();
        store.create_pairing("Phone").unwrap();
        assert_eq!(
            store.consume("not-a-real-secret").unwrap(),
            ConsumeOutcome::NotFound
        );
    }

    #[test]
    fn ttl_expiry_purges_pending_record() {
        let (store, dir) = test_store();
        let created = store.create_pairing("Phone").unwrap();

        // Manually rewrite the file with an already-expired `expires_at`.
        let raw = fs::read_to_string(dir.path().join(PENDING_FILE_NAME)).unwrap();
        let mut file: PendingFile = serde_json::from_str(&raw).unwrap();
        file.pending[0].expires_at = now_secs().saturating_sub(1);
        fs::write(
            dir.path().join(PENDING_FILE_NAME),
            serde_json::to_string_pretty(&file).unwrap(),
        )
        .unwrap();

        assert_eq!(
            store.consume(&created.secret).unwrap(),
            ConsumeOutcome::NotFound
        );
        assert!(store.list_pending().unwrap().is_empty());
    }

    #[test]
    fn max_pending_enforced() {
        let (store, _dir) = test_store();
        for i in 0..PAIRING_PENDING_MAX {
            store.create_pairing(&format!("Device {i}")).unwrap();
        }
        let err = store.create_pairing("One too many").unwrap_err();
        assert!(matches!(err, DevicePairingError::TooManyPending));
    }

    #[test]
    fn lockout_after_repeated_failures() {
        let (store, _dir) = test_store();
        store.create_pairing("Phone").unwrap();
        for _ in 0..PAIRING_FAILED_LIMIT {
            let _ = store.consume("wrong-secret");
        }
        let err = store.consume("wrong-secret").unwrap_err();
        assert!(matches!(err, DevicePairingError::RateLimited));
    }

    #[test]
    fn require_confirm_two_step_flow() {
        let dir = TempDir::new().unwrap();
        let store =
            DevicePairingStore::with_base_dir(dir.path().to_path_buf()).with_require_confirm(true);
        let created = store.create_pairing("Watch").unwrap();

        // First consume: matches, but only marks awaiting_confirm.
        let pairing_id = match store.consume(&created.secret).unwrap() {
            ConsumeOutcome::AwaitingConfirm { pairing_id } => pairing_id,
            other => panic!("expected AwaitingConfirm, got {other:?}"),
        };

        // Not yet approved: repeat consume stays in AwaitingConfirm, does
        // not finalize.
        match store.consume(&created.secret).unwrap() {
            ConsumeOutcome::AwaitingConfirm { .. } => {}
            other => panic!("expected AwaitingConfirm before approval, got {other:?}"),
        }

        let pending = store.list_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert!(pending[0].awaiting_confirm);

        // Admin approves.
        assert!(store.approve(&pairing_id).unwrap());

        // Repeat consume with the same secret now finalizes.
        match store.consume(&created.secret).unwrap() {
            ConsumeOutcome::Consumed { name, .. } => assert_eq!(name, "Watch"),
            other => panic!("expected Consumed after approval, got {other:?}"),
        }

        // Single-use: a further consume fails.
        assert_eq!(
            store.consume(&created.secret).unwrap(),
            ConsumeOutcome::NotFound
        );
    }

    #[test]
    fn approve_unknown_pairing_id_is_noop() {
        let (store, _dir) = test_store();
        store.create_pairing("Phone").unwrap();
        assert!(!store.approve("does-not-exist").unwrap());
    }

    #[test]
    fn list_pending_hides_secret_material() {
        let (store, _dir) = test_store();
        let created = store.create_pairing("Phone").unwrap();
        let pending = store.list_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].pairing_id, created.pairing_id);
        assert_eq!(pending[0].name, "Phone");
    }
}
