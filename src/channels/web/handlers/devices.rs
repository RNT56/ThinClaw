//! Device identity HTTP surface (milestone B1): pairing, device management,
//! and the device's own `/me` view.
//!
//! Design authority: `docs/MOBILE_SECURITY.md` (decisions D-P*/D-T*/D-X*/D-K*,
//! §8 gateway hardening) and `docs/MOBILE_APP.md` (device identity section).
//! The devices *core* (persistence, token issuance, pairing mechanics, scope
//! mapping, audit log) lives in `thinclaw-gateway::web::devices` and is
//! root-independent; this module is the root-owned axum wiring: request/
//! response glue, route-level guards (rate limiting, body limits, admin-vs-
//! device principal checks), and TLS/instance-id integration.
//!
//! Route summary (registered in `src/channels/web/server.rs`):
//! - `POST /api/devices/pair/start` — admin-only, builds the QR payload.
//! - `GET  /api/devices/pair/pending` — admin-only.
//! - `POST /api/devices/pair/complete` — **public**, protected by the
//!   one-time secret/code, a dedicated rate limiter, and a 4 KB body limit.
//! - `POST /api/devices/pair/{id}/approve` — admin-only (`require_confirm`).
//! - `GET  /api/devices`, `POST /api/devices/{id}/rename|revoke|rotate` —
//!   admin-only; device principals are already excluded from these routes by
//!   `thinclaw_gateway::web::devices::required_scope` returning `None` for
//!   them (enforced in `auth_middleware`), so no extra check is needed here.
//! - `GET  /api/devices/me` — device-token-only (`devices:self` scope);
//!   requires the `DeviceContext` request extension the auth middleware
//!   attaches for device-authenticated requests.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Serialize;

use crate::channels::web::server::GatewayState;
use thinclaw_gateway::web::devices::{
    ConsumeOutcome, DeviceAuditEvent, DeviceAuditLog, DeviceInfo, DeviceListResponse,
    DevicePairingStore, DevicePlatform, DeviceScope, DeviceStore, PairCompleteRequest,
    PairCompleteResponse, PairStartResponse, PendingPairInfo, PendingPairListResponse,
    QrPairingPayload, RenameDeviceRequest, RotateTokenResponse,
};
use thinclaw_gateway::web::identity::DeviceContext;

/// Env var fallback for `device_pairing.require_confirm` when no database
/// (and hence no `SettingsPort`-backed settings store) is attached to this
/// gateway. Documented here and in `docs/MOBILE_SECURITY.md`. Settings-store
/// values (key `device_pairing_require_confirm`) take precedence when a
/// store is available; this env var covers the store-less / early-boot case.
const REQUIRE_CONFIRM_ENV: &str = "THINCLAW_DEVICE_PAIRING_REQUIRE_CONFIRM";
const REQUIRE_CONFIRM_SETTINGS_KEY: &str = "device_pairing_require_confirm";

/// 4 KB body limit for the public `pair/complete` endpoint (§8 hardening
/// item 1). Applied as a dedicated `DefaultBodyLimit` layer in `server.rs`,
/// scoped to just this route.
pub(crate) const PAIR_COMPLETE_BODY_LIMIT_BYTES: usize = 4 * 1024;

fn device_store() -> DeviceStore {
    DeviceStore::new()
}

fn pairing_store(require_confirm: bool) -> DevicePairingStore {
    DevicePairingStore::new().with_require_confirm(require_confirm)
}

fn audit_log() -> DeviceAuditLog {
    DeviceAuditLog::new()
}

fn device_info_list(records: &[thinclaw_gateway::web::devices::DeviceRecord]) -> Vec<DeviceInfo> {
    records.iter().map(DeviceInfo::from).collect()
}

fn device_store_error(
    error: thinclaw_gateway::web::devices::DeviceStoreError,
) -> (StatusCode, String) {
    use thinclaw_gateway::web::devices::DeviceStoreError;
    match error {
        DeviceStoreError::NotFound(id) => {
            (StatusCode::NOT_FOUND, format!("device not found: {id}"))
        }
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

fn pairing_error(
    error: thinclaw_gateway::web::devices::DevicePairingError,
) -> (StatusCode, String) {
    use thinclaw_gateway::web::devices::DevicePairingError;
    match error {
        DevicePairingError::TooManyPending => (
            StatusCode::TOO_MANY_REQUESTS,
            "too many pending pairing requests; try again later".to_string(),
        ),
        DevicePairingError::RateLimited => (
            StatusCode::TOO_MANY_REQUESTS,
            "too many failed pairing attempts; try again later".to_string(),
        ),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

/// Read (or create) the stable gateway instance id persisted at
/// `~/.thinclaw/instance-id`. Used as the QR payload's `iid` field so a
/// rediscovered/reconnected endpoint can be recognized independent of which
/// URL it was reached at (D-X3 — discovery is a locator, never an
/// authenticator; the instance id is part of what must match before a
/// credential is sent).
fn resolve_or_create_instance_id() -> std::io::Result<String> {
    let path = thinclaw_platform::resolve_thinclaw_home().join("instance-id");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let instance_id = uuid::Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Single-writer-at-creation-time: no fs4 lock needed here (unlike the
    // devices/pairing stores, which are read-modify-written on every
    // request). A tmp+rename keeps a concurrent first-boot race from ever
    // observing a half-written file.
    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, &instance_id)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(instance_id)
}

/// Resolve `device_pairing.require_confirm`: settings store (when a database
/// is attached) takes precedence, falling back to `THINCLAW_DEVICE_PAIRING_REQUIRE_CONFIRM`
/// (`"1"`/`"true"` = on) when no store is available or the key is unset.
async fn resolve_require_confirm(state: &GatewayState) -> bool {
    if let Some(store) = state.store.as_ref() {
        use thinclaw_gateway::web::ports::SettingsPort;
        if let Ok(snapshot) = state.load_settings(&state.user_id).await
            && let Some(value) = snapshot.values.get(REQUIRE_CONFIRM_SETTINGS_KEY)
        {
            if let Some(b) = value.as_bool() {
                return b;
            }
            if let Some(s) = value.as_str() {
                return s.eq_ignore_ascii_case("true") || s == "1";
            }
        }
        // Store attached but key unset: still fall through to the env var
        // rather than assuming `false`, so operators without a settings UI
        // path can still opt in.
        let _ = store;
    }

    std::env::var(REQUIRE_CONFIRM_ENV)
        .ok()
        .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
}

// --- POST /api/devices/pair/start (admin-only) ---

#[utoipa::path(
    post,
    path = "/api/devices/pair/start",
    tag = "devices",
    responses(
        (status = 200, description = "QR pairing payload issued", body = PairStartResponse),
        (status = 401, description = "Missing or invalid gateway bearer token"),
        (status = 403, description = "Device principals cannot administer pairing"),
        (status = 429, description = "Too many pending pairing requests"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn devices_pair_start_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<PairStartResponse>, (StatusCode, String)> {
    let require_confirm = resolve_require_confirm(state.as_ref()).await;
    let pairing = pairing_store(require_confirm)
        .create_pairing("New device")
        .map_err(pairing_error)?;

    let _ = audit_log().record(DeviceAuditEvent::PairingCreated, None, None, None);

    let instance_id = resolve_or_create_instance_id()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (urls, fingerprint) = pairing_qr_transport_info().await;

    let qr_payload = QrPairingPayload {
        v: 1,
        urls,
        fp: fingerprint,
        iid: instance_id,
        name: "ThinClaw Gateway".to_string(),
        sec: pairing.secret.clone(),
        exp: pairing.expires_at as i64,
    };
    let qr_json = serde_json::to_string(&qr_payload)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let qr_uri = format!(
        "thinclaw://pair?d={}",
        base64_url_encode(qr_json.as_bytes())
    );

    Ok(Json(PairStartResponse {
        qr_payload,
        qr_uri,
        human_code: pairing.code,
        expires_at: pairing.expires_at as i64,
        pairing_id: pairing.pairing_id,
    }))
}

fn base64_url_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Resolve the URLs + TLS SPKI fingerprint to embed in the pairing QR.
/// `gateway-tls` is a default-on feature (see `docs/BUILD_PROFILES.md`) but
/// not universal (e.g. `edge` builds) — without it, the gateway has no TLS
/// listener to advertise, so pairing degrades to the no-fingerprint /
/// no-URLs case (the `vpn-http`-style QR shape D-P1/D-X1 already reserve
/// `fp: None` for). Starts the TLS listener (auto mode) on first pairing,
/// per `docs/MOBILE_SECURITY.md` D-X1.
#[cfg(feature = "gateway-tls")]
async fn pairing_qr_transport_info() -> (Vec<String>, Option<String>) {
    if let Err(e) = crate::channels::web::tls::ensure_started().await {
        tracing::warn!("Failed to start gateway TLS listener during pairing: {}", e);
    }

    let base_dir = thinclaw_platform::resolve_thinclaw_home();
    let fingerprint = crate::channels::web::tls::TlsMaterial::load_or_generate(&base_dir)
        .ok()
        .map(|material| material.fingerprint_base64url());
    let port = crate::channels::web::tls::tls_port();
    (
        crate::channels::web::tls::advertised_urls(port),
        fingerprint,
    )
}

#[cfg(not(feature = "gateway-tls"))]
async fn pairing_qr_transport_info() -> (Vec<String>, Option<String>) {
    (Vec::new(), None)
}

// --- GET /api/devices/pair/pending (admin-only) ---

#[utoipa::path(
    get,
    path = "/api/devices/pair/pending",
    tag = "devices",
    responses(
        (status = 200, description = "Pending device-pairing requests", body = PendingPairListResponse),
        (status = 401, description = "Missing or invalid gateway bearer token"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn devices_pair_pending_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<PendingPairListResponse>, (StatusCode, String)> {
    let require_confirm = resolve_require_confirm(state.as_ref()).await;
    let pending = pairing_store(require_confirm)
        .list_pending()
        .map_err(pairing_error)?;

    Ok(Json(PendingPairListResponse {
        pending: pending
            .into_iter()
            .map(|p| PendingPairInfo {
                pairing_id: p.pairing_id,
                name: p.name,
                created_at: p.created_at.to_string(),
                expires_at: p.expires_at as i64,
                awaiting_confirm: p.awaiting_confirm,
            })
            .collect(),
    }))
}

// --- POST /api/devices/pair/complete (public) ---

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub(crate) struct PairPendingConfirmResponse {
    pub status: &'static str,
    pub pairing_id: String,
}

#[utoipa::path(
    post,
    path = "/api/devices/pair/complete",
    tag = "devices",
    request_body = PairCompleteRequest,
    responses(
        (status = 200, description = "Device paired; token returned exactly once", body = PairCompleteResponse),
        (status = 202, description = "require_confirm mode: awaiting admin approval", body = PairPendingConfirmResponse),
        (status = 400, description = "Unknown or expired pairing credential"),
        (status = 429, description = "Too many pairing attempts"),
    ),
)]
pub(crate) async fn devices_pair_complete_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<PairCompleteRequest>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    use axum::response::IntoResponse;

    if !state.pair_complete_rate_limiter.check() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "too many pairing attempts; try again later".to_string(),
        ));
    }

    let credential = req
        .secret
        .as_deref()
        .or(req.code.as_deref())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "one of `secret` or `code` is required".to_string(),
            )
        })?;

    let require_confirm = resolve_require_confirm(state.as_ref()).await;
    let outcome = pairing_store(require_confirm)
        .consume(credential)
        .map_err(|e| {
            let _ = audit_log().record(DeviceAuditEvent::PairingFailed, None, None, None);
            pairing_error(e)
        })?;

    match outcome {
        ConsumeOutcome::NotFound => {
            let _ = audit_log().record(DeviceAuditEvent::PairingFailed, None, None, None);
            Err((
                StatusCode::BAD_REQUEST,
                "invalid or expired pairing credential".to_string(),
            ))
        }
        ConsumeOutcome::AwaitingConfirm { pairing_id } => {
            let _ = audit_log().record(DeviceAuditEvent::PairingConsumed, None, None, None);
            Ok((
                StatusCode::ACCEPTED,
                Json(PairPendingConfirmResponse {
                    status: "pending_confirm",
                    pairing_id,
                }),
            )
                .into_response())
        }
        ConsumeOutcome::Consumed { name, .. } => {
            let _ = audit_log().record(DeviceAuditEvent::PairingConsumed, None, None, None);
            let device_name = if req.name.trim().is_empty() {
                name
            } else {
                req.name.clone()
            };
            let (record, token) = device_store()
                .insert(
                    device_name,
                    DevicePlatform::parse(&req.platform),
                    DeviceScope::default_grant(),
                    req.pubkey.clone(),
                )
                .map_err(device_store_error)?;
            state
                .device_registry
                .refresh(&record.device_id)
                .await
                .map_err(device_store_error)?;

            let _ = audit_log().record(
                DeviceAuditEvent::DevicePaired,
                Some(&record.device_id),
                Some(&record.token_prefix),
                None,
            );

            let instance_id = resolve_or_create_instance_id()
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            Ok((
                StatusCode::OK,
                Json(PairCompleteResponse {
                    device_id: record.device_id,
                    token,
                    scopes: record.scopes,
                    gateway_instance: instance_id,
                }),
            )
                .into_response())
        }
    }
}

// --- POST /api/devices/pair/{id}/approve (admin-only, strict mode) ---

#[utoipa::path(
    post,
    path = "/api/devices/pair/{id}/approve",
    tag = "devices",
    params(("id" = String, Path, description = "Pairing id from `pair/start`")),
    responses(
        (status = 200, description = "Pairing approved; the device can now finalize with a repeat pair/complete call"),
        (status = 401, description = "Missing or invalid gateway bearer token"),
        (status = 404, description = "Pairing id not found or not awaiting confirmation"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn devices_pair_approve_handler(
    State(state): State<Arc<GatewayState>>,
    Path(pairing_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let require_confirm = resolve_require_confirm(state.as_ref()).await;
    let approved = pairing_store(require_confirm)
        .approve(&pairing_id)
        .map_err(pairing_error)?;

    if !approved {
        return Err((
            StatusCode::NOT_FOUND,
            "pairing id not found or not awaiting confirmation".to_string(),
        ));
    }

    let _ = audit_log().record(DeviceAuditEvent::DeviceApproved, None, None, None);
    Ok(StatusCode::OK)
}

// --- GET /api/devices (admin-only) ---

#[utoipa::path(
    get,
    path = "/api/devices",
    tag = "devices",
    responses(
        (status = 200, description = "All paired devices", body = DeviceListResponse),
        (status = 401, description = "Missing or invalid gateway bearer token"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn devices_list_handler(
    State(_state): State<Arc<GatewayState>>,
) -> Result<Json<DeviceListResponse>, (StatusCode, String)> {
    let records = device_store().list().map_err(device_store_error)?;
    Ok(Json(DeviceListResponse {
        devices: device_info_list(&records),
    }))
}

// --- POST /api/devices/{id}/rename (admin-only) ---

#[utoipa::path(
    post,
    path = "/api/devices/{id}/rename",
    tag = "devices",
    params(("id" = String, Path, description = "Device id")),
    request_body = RenameDeviceRequest,
    responses(
        (status = 200, description = "Renamed device", body = DeviceInfo),
        (status = 401, description = "Missing or invalid gateway bearer token"),
        (status = 404, description = "Device not found"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn devices_rename_handler(
    State(state): State<Arc<GatewayState>>,
    Path(device_id): Path<String>,
    Json(req): Json<RenameDeviceRequest>,
) -> Result<Json<DeviceInfo>, (StatusCode, String)> {
    let record = device_store()
        .rename(&device_id, &req.name)
        .map_err(device_store_error)?;
    state
        .device_registry
        .refresh(&device_id)
        .await
        .map_err(device_store_error)?;
    Ok(Json(DeviceInfo::from(&record)))
}

// --- POST /api/devices/{id}/revoke (admin-only) ---

#[utoipa::path(
    post,
    path = "/api/devices/{id}/revoke",
    tag = "devices",
    params(("id" = String, Path, description = "Device id")),
    responses(
        (status = 200, description = "Revoked device", body = DeviceInfo),
        (status = 401, description = "Missing or invalid gateway bearer token"),
        (status = 404, description = "Device not found"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn devices_revoke_handler(
    State(state): State<Arc<GatewayState>>,
    Path(device_id): Path<String>,
) -> Result<Json<DeviceInfo>, (StatusCode, String)> {
    // `DeviceRegistry::revoke` persists the revocation (via the same store
    // this handler would otherwise call separately) *and* broadcasts the
    // revoked id so live SSE/WS connections tear down synchronously (§8
    // hardening item 5) — use it directly rather than
    // `device_store().revoke()` + a manual `refresh()`.
    let record = state
        .device_registry
        .revoke(&device_id)
        .await
        .map_err(device_store_error)?;

    let _ = audit_log().record(
        DeviceAuditEvent::DeviceRevoked,
        Some(&record.device_id),
        Some(&record.token_prefix),
        None,
    );

    Ok(Json(DeviceInfo::from(&record)))
}

// --- POST /api/devices/{id}/rotate (admin-only) ---

#[utoipa::path(
    post,
    path = "/api/devices/{id}/rotate",
    tag = "devices",
    params(("id" = String, Path, description = "Device id")),
    responses(
        (status = 200, description = "New token issued; returned exactly once", body = RotateTokenResponse),
        (status = 401, description = "Missing or invalid gateway bearer token"),
        (status = 404, description = "Device not found"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn devices_rotate_handler(
    State(state): State<Arc<GatewayState>>,
    Path(device_id): Path<String>,
) -> Result<Json<RotateTokenResponse>, (StatusCode, String)> {
    let (record, token) = device_store()
        .rotate(&device_id)
        .map_err(device_store_error)?;
    state
        .device_registry
        .refresh(&device_id)
        .await
        .map_err(device_store_error)?;

    let _ = audit_log().record(
        DeviceAuditEvent::DeviceTokenRotated,
        Some(&record.device_id),
        Some(&record.token_prefix),
        None,
    );

    Ok(Json(RotateTokenResponse {
        device_id: record.device_id,
        token,
    }))
}

// --- GET /api/devices/me (device token; devices:self scope) ---

#[utoipa::path(
    get,
    path = "/api/devices/me",
    tag = "devices",
    responses(
        (status = 200, description = "The authenticated device's own record", body = DeviceInfo),
        (status = 401, description = "Missing or invalid device token"),
        (status = 403, description = "Not a device-authenticated request"),
        (status = 404, description = "Device record not found"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn devices_me_handler(
    State(_state): State<Arc<GatewayState>>,
    device_ctx: Option<axum::Extension<DeviceContext>>,
) -> Result<Json<DeviceInfo>, (StatusCode, String)> {
    let Some(axum::Extension(ctx)) = device_ctx else {
        // Reached only via shared-token auth (which bypasses scope
        // enforcement entirely) — there is no device identity to report.
        return Err((
            StatusCode::FORBIDDEN,
            "this endpoint requires a device-token-authenticated request".to_string(),
        ));
    };

    let record = device_store()
        .get(&ctx.device_id)
        .map_err(device_store_error)?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("device not found: {}", ctx.device_id),
            )
        })?;

    Ok(Json(DeviceInfo::from(&record)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_or_create_instance_id_is_stable_across_calls() {
        let dir = tempfile::TempDir::new().unwrap();
        // SAFETY (test-only): scoped to this process; no other test reads
        // THINCLAW_HOME concurrently in a way that would race with this one,
        // matching the pattern used by other `#[cfg(test)]` env-var tests in
        // this codebase (e.g. `tls.rs`'s `tls_policy_defaults_to_auto`).
        unsafe {
            std::env::set_var("THINCLAW_HOME", dir.path());
        }

        let first = resolve_or_create_instance_id().unwrap();
        let second = resolve_or_create_instance_id().unwrap();
        assert_eq!(first, second);
        assert!(!first.is_empty());

        unsafe {
            std::env::remove_var("THINCLAW_HOME");
        }
    }

    #[test]
    fn pair_complete_body_limit_matches_hardening_spec() {
        assert_eq!(PAIR_COMPLETE_BODY_LIMIT_BYTES, 4096);
    }
}
