//! End-to-end integration tests for milestone B1 device identity.
//!
//! Modeled on `tests/ws_gateway_integration.rs`: starts a real Axum server
//! on a random loopback port via `start_server`, then drives the full
//! device-pairing flow over plain HTTP (`GATEWAY_TLS=off`).
//!
//! Design authority: `docs/MOBILE_SECURITY.md` (D-P*/D-T*/D-X*, §8 gateway
//! hardening) and `docs/MOBILE_APP.md` (device identity section).
//!
//! State isolation: `DeviceStore`/`DevicePairingStore`/`DeviceAuditLog` all
//! resolve their base directory from `thinclaw_platform::resolve_thinclaw_home()`,
//! which honors `$THINCLAW_HOME` **at the moment each request handler runs**
//! (it is not cached). Because the gateway server for a test keeps serving
//! requests on a background task for the rest of the test body (and `cargo
//! test` runs test fns concurrently within one binary), the env override
//! must stay in effect — and exclusive — for the *entire* test, not just
//! server startup. `HomeOverride` holds the process-wide `ENV_GUARD` mutex
//! for its whole lifetime (must be the first local in each test, dropped
//! last) rather than only around the `set_var` call. This is the same env
//! var `tests/workspace_integration.rs` overrides, but that file has a
//! single test touching it and doesn't need a mutex; this file has many.

use std::net::SocketAddr;
use std::sync::Arc;

use thinclaw::channels::web::server::{GatewayState, start_server};
use thinclaw::channels::web::sse::SseManager;
use thinclaw::channels::web::ws::WsConnectionTracker;
use thinclaw_gateway::web::devices::{DeviceRegistry, DeviceStore};
use tokio::sync::{Mutex, OwnedMutexGuard};

const AUTH_TOKEN: &str = "test-admin-token-98765";
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Serializes access to the process-global `THINCLAW_HOME` env var across
/// tests in this binary. A `tokio::sync::Mutex` (not `std::sync::Mutex`) is
/// used deliberately: its guard is safely held across `.await` points, which
/// `HomeOverride` needs to do for the lifetime of each test.
static ENV_GUARD: std::sync::LazyLock<Arc<Mutex<()>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(())));

/// Holds the `THINCLAW_HOME` override — and the `ENV_GUARD` lock — for the
/// duration of a test, restoring whatever was there before on drop. Must be
/// kept alive (not just its `TempDir`) for as long as the test's gateway
/// server may still be handling requests.
struct HomeOverride {
    _dir: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
    // Held for the guard's entire lifetime: this is what makes the env
    // override exclusive across concurrently-running test fns, not just
    // exclusive around the `set_var` calls.
    _lock: OwnedMutexGuard<()>,
}

impl HomeOverride {
    async fn new() -> Self {
        let lock = Arc::clone(&ENV_GUARD).lock_owned().await;
        let dir = tempfile::tempdir().expect("create temp THINCLAW_HOME");
        let previous = std::env::var_os("THINCLAW_HOME");
        unsafe {
            std::env::set_var("THINCLAW_HOME", dir.path());
            // Guaranteed no-op for this process (gateway-tls off), but keep
            // TLS registry state honest in case another test in this binary
            // started it against a different temp dir.
            std::env::set_var("GATEWAY_TLS", "off");
        }
        Self {
            _dir: dir,
            previous,
            _lock: lock,
        }
    }

    fn path(&self) -> std::path::PathBuf {
        self._dir.path().to_path_buf()
    }
}

impl Drop for HomeOverride {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(v) => std::env::set_var("THINCLAW_HOME", v),
                None => std::env::remove_var("THINCLAW_HOME"),
            }
        }
    }
}

/// Start an isolated gateway on a random port with its own `THINCLAW_HOME`.
/// Returns the bound address, the `HomeOverride` guard (keep alive for the
/// duration of the test — dropping it early re-exposes the shared
/// `THINCLAW_HOME` to this test's still-running background server task),
/// and the shared state.
async fn start_isolated_server() -> (SocketAddr, HomeOverride, Arc<GatewayState>) {
    let home = HomeOverride::new().await;

    // Mark TLS explicitly inactive for this process before the server binds
    // — mirrors what `start_server` itself does when `GATEWAY_TLS=off`, and
    // guards the pairing handler's lazy `ensure_started()` call too.
    thinclaw::channels::web::tls::mark_inactive().await;

    let device_store = DeviceStore::with_base_dir(home.path());
    let device_registry = Arc::new(
        DeviceRegistry::load(device_store)
            .await
            .expect("load empty device store"),
    );

    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: None,
        session_manager: Some(Arc::new(thinclaw::agent::SessionManager::new())),
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        context_manager: None,
        scheduler: tokio::sync::RwLock::new(None),
        user_id: "test-user".to_string(),
        actor_id: "test-actor".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        llm_runtime: None,
        skill_registry: None,
        skill_catalog: None,
        skill_remote_hub: None,
        skill_quarantine: None,
        chat_rate_limiter: thinclaw::channels::web::rate_limiter::RateLimiter::new(30, 60),
        pair_complete_rate_limiter: thinclaw::channels::web::rate_limiter::RateLimiter::new(
            10, 300,
        ),
        registry_entries: Vec::new(),
        cost_guard: None,
        cost_tracker: None,
        response_cache: None,
        startup_time: std::time::Instant::now(),
        restart_requested: std::sync::atomic::AtomicBool::new(false),
        routine_engine: None,
        repo_project_supervisor: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        secrets_store: None,
        channel_manager: None,
        hooks: None,
        device_registry,
        pending_approvals: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound_addr = start_server(addr, state.clone(), AUTH_TOKEN.to_string(), vec![])
        .await
        .expect("Failed to start test server");

    (bound_addr, home, state)
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

async fn pair_start(addr: SocketAddr) -> serde_json::Value {
    let resp = client()
        .post(format!("http://{addr}/api/devices/pair/start"))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .send()
        .await
        .expect("pair/start request failed");
    assert_eq!(resp.status(), 200, "pair/start should succeed");
    resp.json().await.expect("pair/start body should be JSON")
}

async fn pair_complete(addr: SocketAddr, secret: &str, name: &str) -> reqwest::Response {
    client()
        .post(format!("http://{addr}/api/devices/pair/complete"))
        .json(&serde_json::json!({
            "secret": secret,
            "name": name,
            "platform": "ios",
        }))
        .send()
        .await
        .expect("pair/complete request failed")
}

/// Pair a fresh phone and return its `(device_id, token)`.
async fn pair_phone(addr: SocketAddr, name: &str) -> (String, String) {
    let start_body = pair_start(addr).await;
    let secret = start_body["qr_payload"]["sec"].as_str().unwrap();
    let resp = pair_complete(addr, secret, name).await;
    assert_eq!(resp.status(), 200, "pair/complete should succeed");
    let body: serde_json::Value = resp.json().await.unwrap();
    (
        body["device_id"].as_str().unwrap().to_string(),
        body["token"].as_str().unwrap().to_string(),
    )
}

/// Mint a watch companion of `parent_token`, returning the raw response.
async fn create_companion(addr: SocketAddr, parent_token: &str, name: &str) -> reqwest::Response {
    client()
        .post(format!("http://{addr}/api/devices/me/companions"))
        .header("Authorization", format!("Bearer {parent_token}"))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .expect("companion create request failed")
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_pair_start_returns_code_and_secret() {
    let (addr, _home, _state) = start_isolated_server().await;

    let body = pair_start(addr).await;
    assert!(
        body["qr_payload"]["sec"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "expected a non-empty one-time secret, got: {body}"
    );
    assert!(
        body["human_code"].as_str().is_some_and(|s| s.len() == 8),
        "expected an 8-char human code, got: {body}"
    );
    assert!(body["pairing_id"].as_str().is_some());
    assert!(body["expires_at"].as_i64().is_some());
}

#[tokio::test]
async fn test_pair_complete_issues_token_and_hashes_at_rest() {
    let (addr, home, _state) = start_isolated_server().await;

    let start_body = pair_start(addr).await;
    let secret = start_body["qr_payload"]["sec"].as_str().unwrap();

    let resp = pair_complete(addr, secret, "My Phone").await;
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();

    let token = body["token"].as_str().expect("token field present");
    assert!(token.starts_with("tcd_"), "token should have tcd_ prefix");
    assert!(body["device_id"].as_str().is_some());
    assert!(body["gateway_instance"].as_str().is_some());

    // Persisted devices.json must contain the SHA-256 hash of the token,
    // but never the raw token string.
    let devices_json_path = home.path().join("devices.json");
    let raw = tokio::fs::read_to_string(&devices_json_path)
        .await
        .expect("devices.json should exist after pairing");
    assert!(
        !raw.contains(token),
        "raw device token must never be persisted to devices.json"
    );

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let expected_hash = hex::encode(hasher.finalize());
    assert!(
        raw.contains(&expected_hash),
        "devices.json should contain the token's SHA-256 hash"
    );
}

#[tokio::test]
async fn test_device_token_can_read_chat_threads_and_devices_me() {
    let (addr, _home, _state) = start_isolated_server().await;

    let start_body = pair_start(addr).await;
    let secret = start_body["qr_payload"]["sec"].as_str().unwrap();
    let complete_resp = pair_complete(addr, secret, "My Phone").await;
    assert_eq!(complete_resp.status(), 200);
    let complete_body: serde_json::Value = complete_resp.json().await.unwrap();
    let token = complete_body["token"].as_str().unwrap();
    let device_id = complete_body["device_id"].as_str().unwrap();

    // GET /api/chat/threads
    let resp = client()
        .get(format!("http://{addr}/api/chat/threads"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("chat/threads request failed");
    assert_eq!(resp.status(), 200, "device token should read chat threads");

    // GET /api/devices/me
    let resp = client()
        .get(format!("http://{addr}/api/devices/me"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("devices/me request failed");
    assert_eq!(
        resp.status(),
        200,
        "device token should read its own record"
    );
    let me_body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(me_body["device_id"].as_str().unwrap(), device_id);
    assert_eq!(me_body["name"].as_str().unwrap(), "My Phone");
    // Never leaks token/hash material via the /me view.
    assert!(me_body.get("token_hash").is_none());
    assert!(me_body.get("token").is_none());
}

#[tokio::test]
async fn test_device_token_forbidden_on_settings_and_admin_revoke() {
    let (addr, _home, _state) = start_isolated_server().await;

    let start_body = pair_start(addr).await;
    let secret = start_body["qr_payload"]["sec"].as_str().unwrap();
    let complete_resp = pair_complete(addr, secret, "My Phone").await;
    let complete_body: serde_json::Value = complete_resp.json().await.unwrap();
    let token = complete_body["token"].as_str().unwrap();
    let device_id = complete_body["device_id"].as_str().unwrap();

    // /api/settings is never grantable to a device token — generic 403.
    let resp = client()
        .get(format!("http://{addr}/api/settings"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("settings request failed");
    assert_eq!(resp.status(), 403);
    let body_text = resp.text().await.unwrap();
    assert_eq!(
        body_text, "Forbidden",
        "device principal must get the generic scope-denial body"
    );

    // Device management (revoke) is never device-scoped either, even for
    // the device's own id.
    let resp = client()
        .post(format!("http://{addr}/api/devices/{device_id}/revoke"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("revoke-as-device request failed");
    assert!(
        resp.status() == reqwest::StatusCode::FORBIDDEN
            || resp.status() == reqwest::StatusCode::UNAUTHORIZED,
        "expected 403/401 for device-authenticated revoke attempt, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn test_device_token_via_query_param_is_rejected() {
    let (addr, _home, _state) = start_isolated_server().await;

    let start_body = pair_start(addr).await;
    let secret = start_body["qr_payload"]["sec"].as_str().unwrap();
    let complete_resp = pair_complete(addr, secret, "My Phone").await;
    let complete_body: serde_json::Value = complete_resp.json().await.unwrap();
    let token = complete_body["token"].as_str().unwrap();

    let resp = client()
        .get(format!("http://{addr}/api/chat/threads?token={token}"))
        .send()
        .await
        .expect("query-param request failed");
    assert_eq!(
        resp.status(),
        401,
        "device tokens must be header-only; ?token= must be rejected"
    );
}

#[tokio::test]
async fn test_admin_revoke_invalidates_device_token() {
    let (addr, _home, state) = start_isolated_server().await;

    let start_body = pair_start(addr).await;
    let secret = start_body["qr_payload"]["sec"].as_str().unwrap();
    let complete_resp = pair_complete(addr, secret, "My Phone").await;
    let complete_body: serde_json::Value = complete_resp.json().await.unwrap();
    let token = complete_body["token"].as_str().unwrap();
    let device_id = complete_body["device_id"].as_str().unwrap();

    // Sanity: token works before revocation.
    let resp = client()
        .get(format!("http://{addr}/api/devices/me"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Admin (shared-token) revoke.
    let resp = client()
        .post(format!("http://{addr}/api/devices/{device_id}/revoke"))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .send()
        .await
        .expect("admin revoke request failed");
    assert_eq!(resp.status(), 200, "admin revoke should succeed");

    // Give the revocation broadcast/registry refresh a moment (best-effort;
    // `revoke` awaits the store write + in-memory refresh synchronously, but
    // leave a small grace window for the HTTP round trip).
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let resp = client()
                .get(format!("http://{addr}/api/devices/me"))
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
                .unwrap();
            if resp.status() == 401 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("device token should become invalid after admin revoke");

    // Also confirm via the in-process registry directly (belt-and-suspenders).
    assert!(state.device_registry.authenticate(token).await.is_none());
}

#[tokio::test]
async fn test_device_token_registers_push_but_admin_token_is_forbidden() {
    let (addr, _home, _state) = start_isolated_server().await;

    let start_body = pair_start(addr).await;
    let secret = start_body["qr_payload"]["sec"].as_str().unwrap();
    let complete_resp = pair_complete(addr, secret, "My Phone").await;
    assert_eq!(complete_resp.status(), 200);
    let complete_body: serde_json::Value = complete_resp.json().await.unwrap();
    let token = complete_body["token"].as_str().unwrap();

    // Device token succeeds: PUT /api/devices/me/push.
    let resp = client()
        .put(format!("http://{addr}/api/devices/me/push"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "apns_token": "apns-device-token-abc",
            "environment": "production",
        }))
        .send()
        .await
        .expect("device push register request failed");
    assert_eq!(
        resp.status(),
        200,
        "device token should register its own push token"
    );

    // The shared admin token reaches `/api/devices/me/push` (that prefix maps
    // to `devices:self`, and the shared token bypasses scope enforcement),
    // but it carries no `DeviceContext`, so the handler's own guard returns
    // 403 — `/me` routes are device-scoped by design, never admin-usable.
    let resp = client()
        .put(format!("http://{addr}/api/devices/me/push"))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .json(&serde_json::json!({
            "apns_token": "apns-device-token-abc",
            "environment": "production",
        }))
        .send()
        .await
        .expect("admin push register request failed");
    assert_eq!(
        resp.status(),
        403,
        "shared admin token must not register a device's push token (no device principal)"
    );
}

#[tokio::test]
async fn test_admin_revoke_clears_registered_push_token() {
    let (addr, home, _state) = start_isolated_server().await;

    let start_body = pair_start(addr).await;
    let secret = start_body["qr_payload"]["sec"].as_str().unwrap();
    let complete_resp = pair_complete(addr, secret, "My Phone").await;
    let complete_body: serde_json::Value = complete_resp.json().await.unwrap();
    let token = complete_body["token"].as_str().unwrap();
    let device_id = complete_body["device_id"].as_str().unwrap();

    // Register a push token as the device.
    let resp = client()
        .put(format!("http://{addr}/api/devices/me/push"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "apns_token": "apns-device-token-abc",
            "environment": "development",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Admin revoke.
    let resp = client()
        .post(format!("http://{addr}/api/devices/{device_id}/revoke"))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .send()
        .await
        .expect("admin revoke request failed");
    assert_eq!(resp.status(), 200);

    // The push registration must be gone (cleared atomically with the revoke
    // write) — read it back through the store directly (same on-disk file the
    // gateway's registry is backed by).
    let store = DeviceStore::with_base_dir(home.path());
    let record = store.get(device_id).unwrap().unwrap();
    assert!(
        record.apns.is_none(),
        "revoke must clear the device's APNs registration"
    );
    assert!(record.revoked_at.is_some());
}

#[tokio::test]
async fn test_repeated_pair_complete_with_garbage_secrets_locks_out() {
    let (addr, _home, _state) = start_isolated_server().await;

    // Create one pending pairing so there's at least one real record in the
    // store (not required for the lockout, which is keyed on failed
    // attempts alone, but keeps this close to a realistic sequence).
    let _ = pair_start(addr).await;

    // thinclaw_gateway::web::devices::PAIRING_FAILED_LIMIT failed attempts
    // are allowed within the window; the next one is rate-limited (429).
    use thinclaw_gateway::web::devices::PAIRING_FAILED_LIMIT;

    let mut last_status = reqwest::StatusCode::OK;
    for i in 0..(PAIRING_FAILED_LIMIT + 1) {
        let resp = pair_complete(addr, &format!("garbage-secret-{i}"), "Attacker").await;
        last_status = resp.status();
        if i < PAIRING_FAILED_LIMIT {
            assert_eq!(
                last_status,
                reqwest::StatusCode::BAD_REQUEST,
                "attempt {i} should be a plain invalid-credential rejection"
            );
        }
    }

    assert_eq!(
        last_status,
        reqwest::StatusCode::TOO_MANY_REQUESTS,
        "attempt after the failure limit should be locked out (429)"
    );
}

// ============================================================================
// Companion devices (milestone M4)
// ============================================================================

#[tokio::test]
async fn test_companion_mint_has_reduced_scopes_and_parent_link() {
    let (addr, _home, _state) = start_isolated_server().await;
    let (parent_id, parent_token) = pair_phone(addr, "My Phone").await;

    let resp = create_companion(addr, &parent_token, "My Watch").await;
    assert_eq!(resp.status(), 200, "companion mint should succeed");
    let body: serde_json::Value = resp.json().await.unwrap();

    let companion_token = body["token"].as_str().expect("companion token present");
    assert!(companion_token.starts_with("tcd_"));
    assert_ne!(
        companion_token, parent_token,
        "companion gets its own token"
    );
    assert_eq!(body["parent_device_id"].as_str().unwrap(), parent_id);

    // Reduced scopes: chat + approvals only, never jobs:read / devices:self.
    let scopes: Vec<String> = body["scopes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    assert!(scopes.contains(&"chat".to_string()));
    assert!(scopes.contains(&"approvals".to_string()));
    assert!(!scopes.contains(&"jobs:read".to_string()));
    assert!(!scopes.contains(&"devices:self".to_string()));

    // The companion can read chat (its `chat` scope).
    let companion_id = body["device_id"].as_str().unwrap();
    let resp = client()
        .get(format!("http://{addr}/api/chat/threads"))
        .header("Authorization", format!("Bearer {companion_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "companion may read chat");

    // The parent lists its companion.
    let resp = client()
        .get(format!("http://{addr}/api/devices/me/companions"))
        .header("Authorization", format!("Bearer {parent_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let list: serde_json::Value = resp.json().await.unwrap();
    let companions = list["companions"].as_array().unwrap();
    assert_eq!(companions.len(), 1);
    assert_eq!(companions[0]["device_id"].as_str().unwrap(), companion_id);
    assert_eq!(
        companions[0]["parent_device_id"].as_str().unwrap(),
        parent_id
    );
}

#[tokio::test]
async fn test_companion_forbidden_on_jobs_and_companion_management() {
    let (addr, _home, _state) = start_isolated_server().await;
    let (_parent_id, parent_token) = pair_phone(addr, "My Phone").await;
    let resp = create_companion(addr, &parent_token, "My Watch").await;
    let body: serde_json::Value = resp.json().await.unwrap();
    let companion_token = body["token"].as_str().unwrap();

    // jobs:read is not in the companion grant → 403.
    let resp = client()
        .get(format!("http://{addr}/api/jobs"))
        .header("Authorization", format!("Bearer {companion_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "companion must not reach jobs:read"
    );

    // devices:self is not granted → the companion cannot mint sub-companions
    // or manage devices.
    let resp = client()
        .post(format!("http://{addr}/api/devices/me/companions"))
        .header("Authorization", format!("Bearer {companion_token}"))
        .json(&serde_json::json!({ "name": "Sub" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "companion must not mint sub-companions (no devices:self)"
    );

    // And an admin route stays forbidden too.
    let resp = client()
        .get(format!("http://{addr}/api/devices"))
        .header("Authorization", format!("Bearer {companion_token}"))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == reqwest::StatusCode::FORBIDDEN
            || resp.status() == reqwest::StatusCode::UNAUTHORIZED,
        "companion must not reach the admin device list"
    );
}

#[tokio::test]
async fn test_parent_revoke_cascades_to_companion_token() {
    let (addr, _home, _state) = start_isolated_server().await;
    let (parent_id, parent_token) = pair_phone(addr, "My Phone").await;
    let resp = create_companion(addr, &parent_token, "My Watch").await;
    let body: serde_json::Value = resp.json().await.unwrap();
    let companion_token = body["token"].as_str().unwrap().to_string();

    // Companion authenticates before revocation.
    let resp = client()
        .get(format!("http://{addr}/api/chat/threads"))
        .header("Authorization", format!("Bearer {companion_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Admin revokes the PARENT.
    let resp = client()
        .post(format!("http://{addr}/api/devices/{parent_id}/revoke"))
        .header("Authorization", format!("Bearer {AUTH_TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // The companion token is now invalid (cascade) → 401.
    let resp = client()
        .get(format!("http://{addr}/api/chat/threads"))
        .header("Authorization", format!("Bearer {companion_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "revoking the parent must cascade-revoke the companion token"
    );
}

#[tokio::test]
async fn test_parent_can_revoke_its_own_companion() {
    let (addr, _home, _state) = start_isolated_server().await;
    let (_parent_id, parent_token) = pair_phone(addr, "My Phone").await;
    let resp = create_companion(addr, &parent_token, "My Watch").await;
    let body: serde_json::Value = resp.json().await.unwrap();
    let companion_id = body["device_id"].as_str().unwrap().to_string();
    let companion_token = body["token"].as_str().unwrap().to_string();

    // Parent revokes its companion via DELETE.
    let resp = client()
        .delete(format!(
            "http://{addr}/api/devices/me/companions/{companion_id}"
        ))
        .header("Authorization", format!("Bearer {parent_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "parent may revoke its own companion");

    // Companion token no longer authenticates.
    let resp = client()
        .get(format!("http://{addr}/api/chat/threads"))
        .header("Authorization", format!("Bearer {companion_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);

    // The parent itself is unaffected.
    let resp = client()
        .get(format!("http://{addr}/api/devices/me"))
        .header("Authorization", format!("Bearer {parent_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "parent stays valid after revoking companion"
    );
}

#[tokio::test]
async fn test_parent_cannot_revoke_another_parents_companion() {
    let (addr, _home, _state) = start_isolated_server().await;
    let (_parent_a_id, parent_a_token) = pair_phone(addr, "Phone A").await;
    let (_parent_b_id, parent_b_token) = pair_phone(addr, "Phone B").await;

    let resp = create_companion(addr, &parent_a_token, "Watch A").await;
    let body: serde_json::Value = resp.json().await.unwrap();
    let companion_a_id = body["device_id"].as_str().unwrap().to_string();
    let companion_a_token = body["token"].as_str().unwrap().to_string();

    // Parent B tries to revoke Parent A's companion → 404 (ownership check).
    let resp = client()
        .delete(format!(
            "http://{addr}/api/devices/me/companions/{companion_a_id}"
        ))
        .header("Authorization", format!("Bearer {parent_b_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "a parent must not revoke another parent's companion"
    );

    // Companion A is still valid.
    let resp = client()
        .get(format!("http://{addr}/api/chat/threads"))
        .header("Authorization", format!("Bearer {companion_a_token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        200,
        "companion A must survive B's failed revoke"
    );
}

#[tokio::test]
async fn test_watch_companion_high_risk_approve_is_forbidden_but_low_risk_ok() {
    use thinclaw_gateway::web::devices::ApprovalRisk;
    use thinclaw_gateway::web::types::PendingApprovalEntry;

    let (addr, _home, state) = start_isolated_server().await;
    let (_parent_id, parent_token) = pair_phone(addr, "My Phone").await;
    let resp = create_companion(addr, &parent_token, "My Watch").await;
    let body: serde_json::Value = resp.json().await.unwrap();
    let companion_token = body["token"].as_str().unwrap().to_string();

    let seed = |request_id: &str, risk: ApprovalRisk| {
        state.pending_approvals.lock().unwrap().insert(
            request_id.to_string(),
            PendingApprovalEntry {
                request_id: request_id.to_string(),
                tool_name: "some_tool".to_string(),
                description: String::new(),
                parameters: "{}".to_string(),
                risk,
                thread_id: None,
                created_at: "2024-01-01T00:00:00+00:00".to_string(),
            },
        );
    };

    // High-risk approve from the watch companion → 403, and the pending entry
    // must survive (decision was not submitted).
    let high_id = uuid::Uuid::new_v4().to_string();
    seed(&high_id, ApprovalRisk::High);
    let resp = client()
        .post(format!("http://{addr}/api/chat/approval"))
        .header("Authorization", format!("Bearer {companion_token}"))
        .json(&serde_json::json!({ "request_id": high_id, "action": "approve" }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "watch companion must be refused a high-risk approval server-side"
    );
    assert!(
        state
            .pending_approvals
            .lock()
            .unwrap()
            .contains_key(&high_id),
        "a rejected high-risk approval must not drop the pending entry"
    );

    // Low-risk approve from the same watch companion passes the watch gate.
    // With no agent loop wired in this harness the submission fails 503, but
    // crucially NOT 403 — the low-risk decision was allowed through the gate,
    // and the pending entry is dropped (decision submitted).
    let low_id = uuid::Uuid::new_v4().to_string();
    seed(&low_id, ApprovalRisk::Low);
    let resp = client()
        .post(format!("http://{addr}/api/chat/approval"))
        .header("Authorization", format!("Bearer {companion_token}"))
        .json(&serde_json::json!({ "request_id": low_id, "action": "approve" }))
        .send()
        .await
        .unwrap();
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::FORBIDDEN,
        "watch companion must be allowed a low-risk approval"
    );
    assert!(
        !state
            .pending_approvals
            .lock()
            .unwrap()
            .contains_key(&low_id),
        "an allowed low-risk approval drops the pending entry"
    );
}
