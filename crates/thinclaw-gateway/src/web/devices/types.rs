//! Device identity domain types and wire DTOs.
//!
//! `DeviceRecord` is the persisted, server-side shape (see
//! [`super::store`]). Everything else in this file is either a request/
//! response DTO for the `/api/devices/*` endpoints described in
//! `docs/MOBILE_APP.md`, or a small supporting enum (`DevicePlatform`,
//! `DeviceScope`).
//!
//! Token and pairing-secret material is never part of any `Serialize` type
//! here except the two responses that hand a freshly issued credential back
//! to the caller once (`PairCompleteResponse`, `RotateTokenResponse`).

use serde::{Deserialize, Serialize};

/// A paired device, as persisted in `~/.thinclaw/devices.json`.
///
/// `token_hash` is `hex(SHA-256(full token string))` — see
/// [`super::store::hash_token`]. The raw token is never stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceRecord {
    /// UUID v4, stringified.
    pub device_id: String,
    pub name: String,
    pub platform: DevicePlatform,
    /// RFC 3339 timestamp.
    pub created_at: String,
    /// RFC 3339 timestamp, updated on each authenticated request (debounced
    /// in-memory by the registry; see [`super::registry::DeviceRegistry`]).
    pub last_seen_at: String,
    /// `hex(SHA-256(token))`. Never the raw token.
    pub token_hash: String,
    /// First 8 characters of the issued token, for display/identification
    /// in the devices UI only — not sufficient to authenticate.
    pub token_prefix: String,
    pub scopes: Vec<DeviceScope>,
    /// Base64-encoded SPKI public key submitted at pairing time (D-P2).
    /// Stored, not yet enforced in v1 (no proof-of-possession signing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
    /// APNs registration for content-free device pushes (milestone B2,
    /// `PUT /api/devices/me/push`). `None` until the device registers a
    /// token. The JSON field name stays `"apns"` so `devices.json` files
    /// written by the earlier placeholder (a `serde_json::Value`) still
    /// deserialize — the placeholder only ever wrote `null`/absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apns: Option<DeviceApnsRegistration>,
    /// Live Activity push tokens, keyed by `activity_id` (milestone B2,
    /// `PUT /api/devices/me/live-activity/{activity_id}`). Bounded to
    /// [`MAX_LIVE_ACTIVITIES_PER_DEVICE`] entries per device; the oldest
    /// (by `updated_at`) is evicted when a new registration would exceed the
    /// cap. Empty maps are skipped so untouched records stay byte-identical.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub live_activities: std::collections::BTreeMap<String, DeviceLiveActivityToken>,
    /// Optional push-to-start token for Live Activities (milestone B2,
    /// `PUT /api/devices/me/live-activity-start-token`). One per device; APNs
    /// uses it to *start* an activity, distinct from the per-activity update
    /// tokens above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_activity_start_token: Option<String>,
    /// Parent device id for a *companion* device (milestone M4). A companion
    /// (e.g. the watch) is a child device minted by an already-paired device
    /// over `POST /api/devices/me/companions`, scoped narrower than its parent
    /// and revoked whenever its parent is revoked (cascade). `None` for a
    /// normal top-level paired device. Serde default so legacy `devices.json`
    /// rows (written before companions existed) still deserialize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_device_id: Option<String>,
    /// RFC 3339 timestamp. `Some` once the device has been revoked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
    /// RFC 3339 timestamp. Optional forced expiry (D-T3); `None` means the
    /// token is long-lived subject only to revocation / inactivity sweep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

impl DeviceRecord {
    /// True if this device is a companion (has a parent device). Companions
    /// are subject to the watch low-risk-only approval rule and are cascade-
    /// revoked with their parent (milestone M4, D-K4).
    pub fn is_companion(&self) -> bool {
        self.parent_device_id.is_some()
    }

    /// True if the device cannot currently authenticate: explicitly revoked
    /// or past its optional `expires_at`.
    pub fn is_blocked(&self, now_rfc3339: &str) -> bool {
        if self.revoked_at.is_some() {
            return true;
        }
        if let Some(expires_at) = &self.expires_at {
            return expires_at.as_str() <= now_rfc3339;
        }
        false
    }
}

/// Maximum number of Live Activity push-token registrations kept per device.
/// A new registration beyond this cap evicts the oldest entry (by
/// `updated_at`) so a runaway client cannot grow `devices.json` without
/// bound.
pub const MAX_LIVE_ACTIVITIES_PER_DEVICE: usize = 16;

/// Persisted APNs registration for a device's content-free pushes (D-N1).
/// Only the device token, target environment, and last-updated timestamp are
/// stored; payload content is never persisted here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceApnsRegistration {
    /// APNs device token (hex string from `didRegisterForRemoteNotifications`).
    pub device_token: String,
    /// APNs environment: `"development"` (sandbox) or `"production"`. Selects
    /// the APNs host when the pusher delivers to this device.
    pub environment: String,
    /// RFC 3339 timestamp of the last registration update.
    pub updated_at: String,
}

/// The kind of Live Activity a registered push token drives (D-N2). Mirrors
/// the two activity surfaces the mobile app runs: an agent run and a
/// background job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DeviceLiveActivityKind {
    AgentRun,
    Job,
}

impl DeviceLiveActivityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceLiveActivityKind::AgentRun => "agent_run",
            DeviceLiveActivityKind::Job => "job",
        }
    }
}

/// Persisted Live Activity update-push token for one `activity_id` (D-N2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceLiveActivityToken {
    /// APNs Live Activity update token.
    pub push_token: String,
    /// What the activity represents (agent run vs. job).
    pub kind: DeviceLiveActivityKind,
    /// The chat thread this activity mirrors, when `kind == AgentRun`. Lets the
    /// first-party notifier route run-progress events (keyed by `thread_id`) to
    /// this activity's per-activity update token. `None` for job activities and
    /// for legacy records written before the association existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// The background job this activity mirrors, when `kind == Job`. Lets the
    /// notifier route job-completion events to this activity. `None` for agent
    /// runs and legacy records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// RFC 3339 timestamp of the last registration update. Also the key used
    /// for oldest-first eviction when the per-device cap is exceeded.
    pub updated_at: String,
}

/// Platform family of a paired device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum DevicePlatform {
    Ios,
    Ipados,
    Watchos,
    Macos,
    /// Anything else (e.g. future platforms, test fixtures). Carries the
    /// raw platform string as reported at pairing time.
    Other(String),
}

impl DevicePlatform {
    pub fn as_str(&self) -> &str {
        match self {
            DevicePlatform::Ios => "ios",
            DevicePlatform::Ipados => "ipados",
            DevicePlatform::Watchos => "watchos",
            DevicePlatform::Macos => "macos",
            DevicePlatform::Other(raw) => raw.as_str(),
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_lowercase().as_str() {
            "ios" => DevicePlatform::Ios,
            "ipados" => DevicePlatform::Ipados,
            "watchos" => DevicePlatform::Watchos,
            "macos" => DevicePlatform::Macos,
            other => DevicePlatform::Other(other.to_string()),
        }
    }
}

/// Device scopes (v1), per `docs/MOBILE_SECURITY.md` D-T4.
///
/// Never grantable to device tokens (enforced by never having a variant
/// here, and by [`super::scopes::required_scope`] returning `None` for
/// those routes): settings, secrets/providers, extensions/skills, memory
/// write, logs, restart, pairing admin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum DeviceScope {
    #[serde(rename = "chat")]
    Chat,
    #[serde(rename = "approvals")]
    Approvals,
    #[serde(rename = "jobs:read")]
    JobsRead,
    #[serde(rename = "devices:self")]
    DevicesSelf,
}

impl DeviceScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceScope::Chat => "chat",
            DeviceScope::Approvals => "approvals",
            DeviceScope::JobsRead => "jobs:read",
            DeviceScope::DevicesSelf => "devices:self",
        }
    }

    /// The default scope grant for a freshly paired device (v1: everything
    /// grantable, since v1 has no per-scope pairing UI yet).
    pub fn default_grant() -> Vec<DeviceScope> {
        vec![
            DeviceScope::Chat,
            DeviceScope::Approvals,
            DeviceScope::JobsRead,
            DeviceScope::DevicesSelf,
        ]
    }

    /// The reduced scope grant for a *companion* device (milestone M4, D-K4).
    ///
    /// A companion (the watch) is minted by an already-paired parent and is
    /// deliberately least-privilege: it can read/send chat and act on
    /// approvals (low-risk only, enforced server-side by device class in the
    /// approve handler), but it gets **no** `jobs:read` and **no**
    /// `devices:self`. Dropping `devices:self` means a companion cannot
    /// enumerate/manage devices or self-register push tokens over HTTP — the
    /// watch is relay-first through the paired phone (there is no Tailscale on
    /// watchOS), so its own device-management surface is intentionally empty.
    /// The parent owns companion lifecycle via `/api/devices/me/companions*`.
    pub fn companion_grant() -> Vec<DeviceScope> {
        vec![DeviceScope::Chat, DeviceScope::Approvals]
    }
}

// --- Pairing DTOs ---

/// QR payload embedded (base64url-json) in `thinclaw://pair?d=<...>`.
///
/// Field names are intentionally short — this travels through a QR code.
/// Unknown fields must be ignored by readers (versioned via `v`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QrPairingPayload {
    /// Payload version.
    pub v: u8,
    /// Candidate gateway URLs (tailnet, `.local`, etc).
    pub urls: Vec<String>,
    /// base64url SHA-256 of the TLS leaf SPKI. Omitted only in `vpn-http` mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fp: Option<String>,
    /// Stable gateway instance id.
    pub iid: String,
    /// Human label for the gateway (shown in the pairing UI).
    pub name: String,
    /// base64url 32-byte one-time pairing secret.
    pub sec: String,
    /// Unix expiry (created + 15 min).
    pub exp: i64,
}

/// Response to `POST /api/devices/pair/start` (admin-only).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PairStartResponse {
    pub qr_payload: QrPairingPayload,
    /// Rendered `thinclaw://pair?d=<base64url(json)>` URI.
    pub qr_uri: String,
    /// Self-contained SVG rendering of `qr_uri` for the authenticated gateway
    /// pairing panel. Optional so newer clients remain compatible with older
    /// gateways and non-visual gateway hosts can omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qr_svg: Option<String>,
    /// Short human-typable code (no-camera fallback), same lockout as the
    /// QR secret.
    pub human_code: String,
    /// Unix expiry, mirrors `qr_payload.exp`.
    pub expires_at: i64,
    /// Internal pairing-record id, used by the `require_confirm` admin
    /// approve call.
    pub pairing_id: String,
}

/// `POST /api/devices/pair/complete` request body (public endpoint,
/// protected only by the one-time secret / human code).
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PairCompleteRequest {
    /// One-time 32-byte secret (base64url) from the QR, OR the short human
    /// code — exactly one of the two redemption paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub name: String,
    pub platform: String,
    /// Base64 SPKI public key (D-P2). Stored, not yet enforced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pubkey: Option<String>,
}

/// `POST /api/devices/pair/complete` response — the only place the raw
/// device token is ever returned.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PairCompleteResponse {
    pub device_id: String,
    /// Raw `tcd_...` token. Returned exactly once.
    pub token: String,
    pub scopes: Vec<DeviceScope>,
    pub gateway_instance: String,
}

// --- Device management DTOs ---

/// Public view of a device (never includes token/hash material).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceInfo {
    pub device_id: String,
    pub name: String,
    pub platform: DevicePlatform,
    pub created_at: String,
    pub last_seen_at: String,
    pub token_prefix: String,
    pub scopes: Vec<DeviceScope>,
    pub has_pubkey: bool,
    /// Parent device id when this is a companion device (milestone M4);
    /// `None` for a normal top-level paired device.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_device_id: Option<String>,
    pub revoked_at: Option<String>,
    pub expires_at: Option<String>,
}

impl From<&DeviceRecord> for DeviceInfo {
    fn from(record: &DeviceRecord) -> Self {
        DeviceInfo {
            device_id: record.device_id.clone(),
            name: record.name.clone(),
            platform: record.platform.clone(),
            created_at: record.created_at.clone(),
            last_seen_at: record.last_seen_at.clone(),
            token_prefix: record.token_prefix.clone(),
            scopes: record.scopes.clone(),
            has_pubkey: record.pubkey.is_some(),
            parent_device_id: record.parent_device_id.clone(),
            revoked_at: record.revoked_at.clone(),
            expires_at: record.expires_at.clone(),
        }
    }
}

/// `GET /api/devices` response.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeviceListResponse {
    pub devices: Vec<DeviceInfo>,
}

/// `POST /api/devices/{id}/rotate` response — the only other place a raw
/// token is returned.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RotateTokenResponse {
    pub device_id: String,
    /// Newly issued raw `tcd_...` token. Returned exactly once.
    pub token: String,
}

/// `POST /api/devices/{id}/rename` request body.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RenameDeviceRequest {
    pub name: String,
}

// --- Push registration DTOs (device-token-only, `devices:self`) ---

/// `PUT /api/devices/me/push` request body: register (or replace) the
/// device's APNs token for content-free pushes.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RegisterPushRequest {
    /// APNs device token (hex from `didRegisterForRemoteNotifications`).
    pub apns_token: String,
    /// APNs environment: `"development"` or `"production"`.
    pub environment: String,
}

/// `PUT /api/devices/me/live-activity/{activity_id}` request body: register
/// (or replace) the Live Activity update-push token for one activity.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RegisterLiveActivityRequest {
    /// APNs Live Activity update token.
    pub push_token: String,
    /// What the activity represents (agent run vs. job).
    pub kind: DeviceLiveActivityKind,
    /// The chat thread this activity mirrors (for `kind == agent_run`). Lets
    /// the gateway route run-progress events to this activity's update token
    /// (D-N2). Optional; omit for job activities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// The background job this activity mirrors (for `kind == job`). Optional;
    /// omit for agent-run activities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
}

/// `PUT /api/devices/me/live-activity-start-token` request body: register (or
/// replace) the device's Live Activity push-to-start token.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RegisterLiveActivityStartTokenRequest {
    /// APNs Live Activity push-to-start token.
    pub push_token: String,
}

// --- Companion device DTOs (milestone M4, device-token-only, `devices:self`) ---

/// `POST /api/devices/me/companions` request body: the current (parent) device
/// mints a reduced-scope companion (e.g. its paired watch). `platform`
/// defaults to `watchos` — the only companion surface in v1 — when omitted.
#[derive(Debug, Clone, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateCompanionRequest {
    pub name: String,
    /// Companion platform; defaults to `"watchos"` when absent or empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
}

impl CreateCompanionRequest {
    /// Resolved platform: the caller-supplied value, or `watchos` when unset
    /// or blank (the only companion surface in v1).
    pub fn resolved_platform(&self) -> DevicePlatform {
        match self.platform.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => DevicePlatform::parse(p),
            _ => DevicePlatform::Watchos,
        }
    }
}

/// `POST /api/devices/me/companions` response — the only place the companion's
/// raw token is ever returned.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateCompanionResponse {
    pub device_id: String,
    /// Raw `tcd_...` companion token. Returned exactly once.
    pub token: String,
    pub scopes: Vec<DeviceScope>,
    /// The parent (minting) device id, echoed for the client's record.
    pub parent_device_id: String,
}

/// `GET /api/devices/me/companions` response: the calling device's own
/// companions (never token/hash material).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CompanionListResponse {
    pub companions: Vec<DeviceInfo>,
}

/// A pending (not-yet-completed) pairing attempt, admin-facing view.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PendingPairInfo {
    pub pairing_id: String,
    pub name: String,
    pub created_at: String,
    pub expires_at: i64,
    /// True once a `pair/complete` call has staged the device awaiting an
    /// admin `approve` call (`require_confirm` mode).
    pub awaiting_confirm: bool,
}

/// `GET /api/devices/pair/pending` response (admin-only).
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PendingPairListResponse {
    pub pending: Vec<PendingPairInfo>,
}
