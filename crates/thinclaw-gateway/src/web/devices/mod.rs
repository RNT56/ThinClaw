//! Device identity core (milestone B1): per-device scoped tokens, pairing,
//! scope enforcement, and audit logging for the ThinClaw mobile surface.
//!
//! Design authority: `docs/MOBILE_SECURITY.md` (decisions D-P*/D-T*/D-X*/D-K*,
//! §8 gateway hardening) and `docs/MOBILE_APP.md` (device identity section).
//! This module is root-independent: persistence, token issuance, pairing,
//! scope mapping, and the in-memory registry all live here so a gateway host
//! can wire them up without pulling in root `thinclaw` crates.
//!
//! Submodules, by responsibility:
//! - [`approval_risk`]: gateway-side approval risk-tier classifier (D-K3),
//!   the single source of truth for the `risk` carried on approval events.
//! - [`types`]: `DeviceRecord`, `DeviceScope`, `DevicePlatform`, and the
//!   request/response DTOs for the `/api/devices/*` endpoints.
//! - [`store`]: persisted `~/.thinclaw/devices.json` CRUD + token issuance.
//! - [`pairing`]: pending-pairing store (`~/.thinclaw/device-pairing.json`),
//!   including the `require_confirm` two-step flow.
//! - [`registry`]: in-memory authentication index over the store, revocation
//!   broadcast, last-seen debounce, inactivity sweep.
//! - [`scopes`]: route -> required-scope precedence map.
//! - [`audit`]: append-only `~/.thinclaw/device-audit.jsonl` writer.
//! - [`push_policy`]: pure `SseEvent` -> content-free `PushDecision` mapping
//!   (throttle, wake budget, Live Activity revisions), consumed by the runtime
//!   first-party push notifier.

pub mod approval_risk;
pub mod audit;
pub mod pairing;
pub mod push_policy;
pub mod registry;
pub mod scopes;
pub mod store;
pub mod types;

pub use approval_risk::{ApprovalRisk, classify as classify_approval_risk};
pub use audit::{DeviceAuditError, DeviceAuditEvent, DeviceAuditLog};
pub use pairing::{
    ConsumeOutcome, CreatedPairing, DevicePairingError, DevicePairingStore, PAIRING_FAILED_LIMIT,
    PAIRING_FAILED_WINDOW_SECS, PAIRING_PENDING_MAX, PAIRING_PENDING_TTL_SECS, PendingPairView,
};
pub use push_policy::{
    BACKGROUND_WAKE_BUDGET, BACKGROUND_WAKE_WINDOW_SECS, DevicePushState,
    LIVE_ACTIVITY_MIN_INTERVAL_SECS, PushDecision, PushKind, decide, decide_background_wake,
    live_activity_start,
};
pub use registry::{DeviceAuth, DeviceRegistry, StreamGuard};
pub use scopes::required_scope;
pub use store::{
    DEVICE_TOKEN_PREFIX, DeviceStore, DeviceStoreError, IssuedToken, hash_token, issue_token,
};
pub use types::{
    CompanionListResponse, CreateCompanionRequest, CreateCompanionResponse, DeviceApnsRegistration,
    DeviceInfo, DeviceListResponse, DeviceLiveActivityKind, DeviceLiveActivityToken,
    DevicePlatform, DeviceRecord, DeviceScope, MAX_LIVE_ACTIVITIES_PER_DEVICE, PairCompleteRequest,
    PairCompleteResponse, PairStartResponse, PendingPairInfo, PendingPairListResponse,
    QrPairingPayload, RegisterLiveActivityRequest, RegisterLiveActivityStartTokenRequest,
    RegisterPushRequest, RenameDeviceRequest, RotateTokenResponse,
};
