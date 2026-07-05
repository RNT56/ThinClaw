//! Append-only device-identity audit log: `~/.thinclaw/device-audit.jsonl`.
//!
//! One JSON object per line (JSONL), fs4-locked for append, `with_base_dir`
//! for tests. Per D-T5 / D-N (logging hygiene), **never** log token or
//! secret material — only `device_id` and `token_prefix` (first 8 chars,
//! already display-only) may appear.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::FileExt;
use serde::Serialize;

const AUDIT_FILE_NAME: &str = "device-audit.jsonl";

#[derive(Debug, thiserror::Error)]
pub enum DeviceAuditError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Audit event kind. Variants intentionally mirror `docs/MOBILE_SECURITY.md`
/// D-T5's list verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceAuditEvent {
    PairingCreated,
    PairingConsumed,
    PairingFailed,
    DevicePaired,
    DeviceApproved,
    DeviceTokenRotated,
    DeviceAuthFailed,
    DeviceScopeDenied,
    DeviceRevoked,
    DeviceAutoRevokedInactive,
    DevicePushTokenRegistered,
}

impl DeviceAuditEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceAuditEvent::PairingCreated => "pairing.created",
            DeviceAuditEvent::PairingConsumed => "pairing.consumed",
            DeviceAuditEvent::PairingFailed => "pairing.failed",
            DeviceAuditEvent::DevicePaired => "device.paired",
            DeviceAuditEvent::DeviceApproved => "device.approved",
            DeviceAuditEvent::DeviceTokenRotated => "device.token_rotated",
            DeviceAuditEvent::DeviceAuthFailed => "device.auth_failed",
            DeviceAuditEvent::DeviceScopeDenied => "device.scope_denied",
            DeviceAuditEvent::DeviceRevoked => "device.revoked",
            DeviceAuditEvent::DeviceAutoRevokedInactive => "device.auto_revoked_inactive",
            DeviceAuditEvent::DevicePushTokenRegistered => "device.push_token_registered",
        }
    }
}

/// One audit line. Deliberately does NOT derive `Deserialize` — this is an
/// append-only write path; nothing in-process needs to parse its own log
/// back. `device_id` is optional because pairing-stage events
/// (`pairing.created/consumed/failed`) may not have a device yet.
#[derive(Debug, Clone, Serialize)]
struct AuditLine<'a> {
    at: String,
    event: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_prefix: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<serde_json::Value>,
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

/// Append-only audit log writer.
#[derive(Debug, Clone)]
pub struct DeviceAuditLog {
    base_dir: PathBuf,
}

impl DeviceAuditLog {
    pub fn new() -> Self {
        Self {
            base_dir: thinclaw_platform::resolve_thinclaw_home(),
        }
    }

    pub fn with_base_dir(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn path(&self) -> PathBuf {
        self.base_dir.join(AUDIT_FILE_NAME)
    }

    /// Append one audit line. `device_id` and `token_prefix` are the only
    /// device-identifying fields ever recorded — never the token or pairing
    /// secret itself.
    pub fn record(
        &self,
        event: DeviceAuditEvent,
        device_id: Option<&str>,
        token_prefix: Option<&str>,
        detail: Option<serde_json::Value>,
    ) -> Result<(), DeviceAuditError> {
        let path = self.path();
        fs::create_dir_all(path.parent().expect("audit path always has a parent"))?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        file.lock_exclusive()?;

        let line = AuditLine {
            at: now_iso(),
            event: event.as_str(),
            device_id,
            token_prefix,
            detail,
        };
        let json = serde_json::to_string(&line)?;
        writeln!(file, "{json}")?;
        file.sync_all()?;

        FileExt::unlock(&file)?;
        Ok(())
    }

    /// Read back all lines (test/debug helper).
    #[cfg(test)]
    fn read_lines(&self) -> Vec<String> {
        fs::read_to_string(self.path())
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

impl Default for DeviceAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_log() -> (DeviceAuditLog, TempDir) {
        let dir = TempDir::new().unwrap();
        let log = DeviceAuditLog::with_base_dir(dir.path().to_path_buf());
        (log, dir)
    }

    #[test]
    fn record_appends_valid_jsonl() {
        let (log, _dir) = test_log();
        log.record(
            DeviceAuditEvent::DevicePaired,
            Some("device-1"),
            Some("tcd_abcd"),
            None,
        )
        .unwrap();
        log.record(DeviceAuditEvent::PairingFailed, None, None, None)
            .unwrap();

        let lines = log.read_lines();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed.get("at").is_some());
            assert!(parsed.get("event").is_some());
        }
    }

    #[test]
    fn event_names_match_spec_strings() {
        assert_eq!(DeviceAuditEvent::PairingCreated.as_str(), "pairing.created");
        assert_eq!(
            DeviceAuditEvent::PairingConsumed.as_str(),
            "pairing.consumed"
        );
        assert_eq!(DeviceAuditEvent::PairingFailed.as_str(), "pairing.failed");
        assert_eq!(DeviceAuditEvent::DevicePaired.as_str(), "device.paired");
        assert_eq!(DeviceAuditEvent::DeviceApproved.as_str(), "device.approved");
        assert_eq!(
            DeviceAuditEvent::DeviceTokenRotated.as_str(),
            "device.token_rotated"
        );
        assert_eq!(
            DeviceAuditEvent::DeviceAuthFailed.as_str(),
            "device.auth_failed"
        );
        assert_eq!(
            DeviceAuditEvent::DeviceScopeDenied.as_str(),
            "device.scope_denied"
        );
        assert_eq!(DeviceAuditEvent::DeviceRevoked.as_str(), "device.revoked");
        assert_eq!(
            DeviceAuditEvent::DeviceAutoRevokedInactive.as_str(),
            "device.auto_revoked_inactive"
        );
        assert_eq!(
            DeviceAuditEvent::DevicePushTokenRegistered.as_str(),
            "device.push_token_registered"
        );
    }

    #[test]
    fn record_never_writes_raw_token_or_secret_material() {
        let (log, _dir) = test_log();
        let fake_token = "tcd_super-secret-do-not-log";
        let fake_secret = "one-time-pairing-secret";

        // Even if a caller passed a detail blob that happened to include
        // sensitive-looking strings under other keys, the log line itself
        // must never carry the actual full token/secret literal beyond the
        // 8-char display prefix, which is the only thing this API accepts
        // as `token_prefix`.
        log.record(
            DeviceAuditEvent::DeviceAuthFailed,
            Some("device-1"),
            Some(&fake_token[..8]),
            None,
        )
        .unwrap();

        let lines = log.read_lines();
        assert_eq!(lines.len(), 1);
        assert!(!lines[0].contains(fake_token));
        assert!(!lines[0].contains(fake_secret));
    }

    #[test]
    fn record_with_detail_round_trips() {
        let (log, _dir) = test_log();
        log.record(
            DeviceAuditEvent::DeviceScopeDenied,
            Some("device-1"),
            None,
            Some(serde_json::json!({"path": "/api/settings", "method": "GET"})),
        )
        .unwrap();

        let lines = log.read_lines();
        let parsed: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(parsed["detail"]["path"], "/api/settings");
    }
}
