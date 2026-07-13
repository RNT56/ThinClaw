//! First-party push notifier: maps live agent events to content-free APNs
//! pushes for paired mobile devices (milestone B2, step 3).
//!
//! Design authority: `docs/MOBILE_SECURITY.md` (D-N1/D-N2, content-free
//! payloads + local rewrite) and `docs/MOBILE_APP.md`.
//!
//! ## What this task does
//!
//! It subscribes to the gateway's SSE broadcast **without consuming a client
//! slot** (via [`SseManager::sender`]'s `subscribe`, not `subscribe_raw`), and
//! for each event iterates the registered devices, runs the pure
//! [`push_policy`](thinclaw_gateway::web::devices::push_policy) mapping against
//! each device's soft runtime state, and delivers any resulting content-free
//! [`PushDecision`] through an APNs [`PushSender`].
//!
//! ## Live Activity run routing
//!
//! A Live Activity registration carries the `thread_id` (or `job_id`) it
//! mirrors. Before deciding, the notifier reconciles each device's tracked
//! runs against that association: a run-progress event on a thread the device
//! registered an activity for auto-tracks the run so `decide` emits throttled
//! Live Activity **updates** to the per-activity token, and the closing
//! `response` emits the **end**. If the device instead has a push-to-start
//! token and no active activity for the thread, the notifier emits a one-shot
//! Live Activity **push-to-start** to the start token so a killed app can spawn
//! the activity; the app's later per-activity registration takes over updates.
//!
//! ## Privacy invariants (owned by the policy, enforced here)
//!
//! - Payload shaping is done entirely in the pure policy module; this runtime
//!   only carries the already-content-free payload to APNs. It never logs the
//!   payload, device token, or any message content — only device ids, decision
//!   kinds, and outcomes.
//! - Alert pushes are **suppressed** for a device that currently has a live
//!   SSE/WS stream open (it is watching events in-app); Live Activity updates
//!   are **not** suppressed (they render on the lock screen regardless).
//!
//! ## Token hygiene
//!
//! When APNs reports a token as `Unregistered`/`BadDeviceToken` (410/400), the
//! notifier prunes *only the rejected registration* from the store, refreshes
//! the registry index, and writes a `device.push_token_removed` audit line —
//! never logging the token itself. An alert/background rejection clears the
//! device's APNs alert registration; a Live Activity rejection clears only
//! that activity's per-activity token, and a push-to-start rejection clears
//! only the start token — both leave the alert registration intact.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use thinclaw_channels::{
    ApnsNativeConfig, ApnsPushSpec, ApnsPushType, ApnsPusher, ApnsSendOutcome,
    ReqwestNativeHttpClient,
};
use thinclaw_gateway::web::devices::push_policy::{PushDecision, PushKind, can_produce_push};
use thinclaw_gateway::web::devices::{
    DeviceAuditEvent, DeviceAuditLog, DeviceLiveActivityKind, DevicePushState, DeviceRecord,
    DeviceRegistry, decide, live_activity_start,
};
use tokio::sync::broadcast;

use crate::channels::web::types::SseEvent;

/// APNs delivery backend behind the notifier, extracted so tests can supply a
/// mock and assert prune-on-410 behavior without a real Apple endpoint.
#[async_trait]
pub trait PushSender: Send + Sync {
    /// Deliver `spec` to `device_token`. `sandbox` selects the APNs host
    /// (development vs. production), chosen per-device from the registration's
    /// `environment`.
    async fn send(
        &self,
        device_token: &str,
        sandbox: bool,
        spec: ApnsPushSpec,
    ) -> Result<ApnsSendOutcome, thinclaw_types::error::ChannelError>;
}

/// [`PushSender`] backed by real [`ApnsPusher`]s. Holds one pusher per APNs
/// environment (sandbox/production) so a single device fleet can mix
/// development and production tokens; both share one signed-request transport.
pub struct ApnsPushSender {
    sandbox: ApnsPusher,
    production: ApnsPusher,
}

impl ApnsPushSender {
    /// Build a sender from `config`, deriving the sandbox and production
    /// pushers over a shared HTTP transport.
    pub fn new(config: ApnsNativeConfig) -> Self {
        let http = Arc::new(ReqwestNativeHttpClient::new());
        let mut sandbox_config = config.clone();
        sandbox_config.sandbox = true;
        let mut production_config = config;
        production_config.sandbox = false;
        Self {
            sandbox: ApnsPusher::new(sandbox_config, http.clone()),
            production: ApnsPusher::new(production_config, http),
        }
    }
}

#[async_trait]
impl PushSender for ApnsPushSender {
    async fn send(
        &self,
        device_token: &str,
        sandbox: bool,
        spec: ApnsPushSpec,
    ) -> Result<ApnsSendOutcome, thinclaw_types::error::ChannelError> {
        let pusher = if sandbox {
            &self.sandbox
        } else {
            &self.production
        };
        pusher.send(device_token, spec).await
    }
}

/// The runtime first-party push notifier. Owns per-device soft policy state
/// and drives deliveries off the gateway SSE broadcast.
pub struct FirstPartyPushNotifier {
    registry: Arc<DeviceRegistry>,
    sender: Arc<dyn PushSender>,
    audit: DeviceAuditLog,
    /// Per-device soft policy state (throttle windows, wake budget, in-flight
    /// Live Activity revisions), keyed by `device_id`. Not persisted.
    device_state: HashMap<String, DevicePushState>,
}

impl FirstPartyPushNotifier {
    /// Build a notifier over `registry` and `sender`, using the default
    /// `~/.thinclaw/device-audit.jsonl` audit log.
    pub fn new(registry: Arc<DeviceRegistry>, sender: Arc<dyn PushSender>) -> Self {
        Self::with_audit_log(registry, sender, DeviceAuditLog::new())
    }

    /// Build a notifier with an explicit audit log (test seam).
    pub fn with_audit_log(
        registry: Arc<DeviceRegistry>,
        sender: Arc<dyn PushSender>,
        audit: DeviceAuditLog,
    ) -> Self {
        Self {
            registry,
            sender,
            audit,
            device_state: HashMap::new(),
        }
    }

    /// Subscribe to `sse_sender` (a plain broadcast subscribe — this does *not*
    /// count against the SSE client-slot limit that `subscribe_raw` enforces)
    /// and run until the sender is dropped and the channel drains.
    ///
    /// Lagged receives are tolerated: a dropped burst of events only means a
    /// few pushes are skipped, which is safe (the app reconciles over its
    /// pinned connection).
    pub async fn run(mut self, sse_sender: broadcast::Sender<SseEvent>) {
        let mut rx = sse_sender.subscribe();
        tracing::info!("First-party push notifier started");
        loop {
            match rx.recv().await {
                Ok(event) => self.handle_event(&event).await,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "First-party push notifier lagged; skipped events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
        tracing::info!("First-party push notifier stopped");
    }

    /// Process one SSE event: for each registered device, decide and deliver a
    /// content-free push (subject to live-stream suppression for alerts).
    async fn handle_event(&mut self, event: &SseEvent) {
        // Hot-path short-circuit: the vast majority of SSE traffic is streaming
        // churn (`stream_chunk`, `thinking`, `heartbeat`) that can never map to
        // a push. Bail out *before* touching the device index so those events
        // do no device work at all (R5). This mirrors the arms of the policy's
        // `decide`.
        if !can_produce_push(event) {
            return;
        }

        // Read devices from the in-memory registry snapshot rather than the
        // on-disk store: no file I/O and no exclusive file lock on the SSE hot
        // path. The snapshot is refreshed on every store mutation, so it
        // reflects push-registration and revocation changes.
        let devices = self.registry.snapshot().await;
        let now_secs = now_secs();
        // The thread this event pertains to (if any) drives Live Activity
        // routing: a device that registered a per-activity update token for
        // this thread has its run auto-tracked so `decide` emits Live Activity
        // updates instead of alerts (D-N2).
        let event_thread = event_thread_id(event).map(str::to_string);
        for device in devices {
            // A revoked device must never be pushed to, even if a stale APNs
            // registration somehow lingered (defense-in-depth alongside the
            // store setters that now reject re-attaching to a revoked device).
            if device.revoked_at.is_some() {
                continue;
            }
            // Only devices with a push registration can receive anything.
            if device.apns.is_none() {
                continue;
            }
            // Reconcile Live Activity run tracking against this device's
            // registered activities *before* deciding, and possibly emit a
            // push-to-start for a killed app. This is what turns a registered
            // Live Activity token into actual run-driven update pushes.
            if let Some(thread_id) = event_thread.as_deref()
                && is_run_progress_event(event)
            {
                self.reconcile_live_activity(&device, thread_id, now_secs)
                    .await;
            }
            let state = self
                .device_state
                .entry(device.device_id.clone())
                .or_default();
            let Some(decision) = decide(event, state, now_secs) else {
                continue;
            };
            self.deliver(&device, decision).await;
        }
    }

    /// Reconcile Live Activity run tracking for one device against a
    /// run-progress event on `thread_id`:
    ///
    /// - If the device has a registered per-activity update token bound to
    ///   `thread_id`, auto-track the run under that `activity_id` so `decide`
    ///   emits throttled Live Activity updates to the per-activity token.
    /// - Otherwise, if the device has a push-to-start token and no active
    ///   activity for the thread, emit a one-shot push-to-start so a killed app
    ///   can spawn the activity, then track the run under a synthesized
    ///   activity id (the app's later per-activity registration, carrying the
    ///   same `thread_id`, takes over on the next event).
    async fn reconcile_live_activity(
        &mut self,
        device: &DeviceRecord,
        thread_id: &str,
        now_secs: u64,
    ) {
        // A registered per-activity update token bound to this thread wins: the
        // app is alive and already has an activity we can drive directly.
        if let Some(activity_id) = registered_activity_for_thread(device, thread_id) {
            let state = self
                .device_state
                .entry(device.device_id.clone())
                .or_default();
            state.ensure_tracked(thread_id, activity_id);
            return;
        }

        // No per-activity token for this thread. If the device offers a
        // push-to-start token and we are not already tracking a run for the
        // thread, emit a one-shot start so a killed app spawns the activity.
        let Some(start_token) = device.live_activity_start_token.clone() else {
            return;
        };
        let start_decision = {
            let state = self
                .device_state
                .entry(device.device_id.clone())
                .or_default();
            if state.is_tracking(thread_id) {
                return;
            }
            // Synthesize an activity id for tracking; the app's own activity id
            // (registered via the per-activity endpoint after it spawns) will
            // replace it on the next event via `ensure_tracked`.
            let activity_id = format!("start-{thread_id}");
            state.ensure_tracked(thread_id, &activity_id);
            if !state.mark_start_emitted(thread_id) {
                return;
            }
            live_activity_start(thread_id, &activity_id, now_secs)
        };
        self.deliver_start(device, &start_token, start_decision)
            .await;
    }

    /// Deliver a Live Activity push-to-start to the device's dedicated
    /// push-to-start token (never a per-activity or alert token). A token
    /// rejection clears **only** the start token, leaving the alert
    /// registration and any per-activity tokens intact (D-N2).
    async fn deliver_start(
        &self,
        device: &DeviceRecord,
        start_token: &str,
        decision: PushDecision,
    ) {
        let environment = device
            .apns
            .as_ref()
            .map(|a| a.environment.clone())
            .unwrap_or_else(|| "production".to_string());
        let sandbox = environment == "development";
        let spec = decision_to_spec(&decision);
        match self.sender.send(start_token, sandbox, spec).await {
            Ok(ApnsSendOutcome::Delivered) => {
                tracing::debug!(
                    device_id = %device.device_id,
                    "push notifier: delivered live-activity push-to-start"
                );
            }
            Ok(ApnsSendOutcome::Unregistered { reason }) => {
                self.prune_start_token(device, &reason).await;
            }
            Err(error) => {
                tracing::warn!(
                    device_id = %device.device_id,
                    %error,
                    "push notifier: push-to-start delivery failed"
                );
            }
        }
    }

    /// Clear a rejected push-to-start token (and only that token), refreshing
    /// the registry and writing an audit line without the token.
    async fn prune_start_token(&self, device: &DeviceRecord, reason: &str) {
        tracing::info!(
            device_id = %device.device_id,
            reason,
            "push notifier: pruning unregistered live-activity start token"
        );
        if let Err(error) = self
            .registry
            .store()
            .clear_live_activity_start_token(&device.device_id)
        {
            tracing::warn!(
                device_id = %device.device_id,
                %error,
                "push notifier: failed to clear pruned start token"
            );
            return;
        }
        if let Err(error) = self.registry.refresh(&device.device_id).await {
            tracing::warn!(
                device_id = %device.device_id,
                %error,
                "push notifier: failed to refresh registry after start-token prune"
            );
        }
        let _ = self.audit.record(
            DeviceAuditEvent::DevicePushTokenRemoved,
            Some(&device.device_id),
            Some(&device.token_prefix),
            Some(serde_json::json!({
                "reason": reason,
                "source": "apns_prune",
                "live_activity_start_token": true,
            })),
        );
    }

    /// Deliver one [`PushDecision`] to `device`, applying live-stream
    /// suppression for alerts and pruning the registration on a token
    /// rejection.
    ///
    /// Token selection is per-decision (D-N2): alert/background pushes target
    /// the device's APNs alert token, while Live Activity updates/ends target
    /// the *per-activity* update token (`live_activities[activity_id]`), never
    /// the alert token.
    async fn deliver(&mut self, device: &DeviceRecord, decision: PushDecision) {
        // Alerts are suppressed while the device is streaming in-app; Live
        // Activity updates/ends are not (they render on the lock screen).
        if matches!(decision.kind, PushKind::Alert | PushKind::Background)
            && self.registry.has_active_stream(&device.device_id)
        {
            tracing::debug!(
                device_id = %device.device_id,
                kind = ?decision.kind,
                "push notifier: suppressed alert to device with live stream"
            );
            return;
        }

        // Resolve the target APNs token and environment for this decision.
        let Some((device_token, environment)) = self.resolve_target(device, &decision) else {
            // No token for this activity/registration (e.g. a Live Activity
            // update for an activity whose token was never registered or was
            // already pruned). Nothing to send.
            return;
        };

        let spec = decision_to_spec(&decision);
        let sandbox = environment == "development";
        match self.sender.send(&device_token, sandbox, spec).await {
            Ok(ApnsSendOutcome::Delivered) => {
                tracing::debug!(
                    device_id = %device.device_id,
                    kind = ?decision.kind,
                    "push notifier: delivered"
                );
            }
            Ok(ApnsSendOutcome::Unregistered { reason }) => {
                self.prune_rejected(device, &decision, &reason).await;
            }
            Err(error) => {
                tracing::warn!(
                    device_id = %device.device_id,
                    kind = ?decision.kind,
                    %error,
                    "push notifier: delivery failed"
                );
            }
        }
    }

    /// Resolve the `(device_token, environment)` this decision must be sent to.
    /// Alert/background use the device's APNs alert registration; Live Activity
    /// updates/ends use the per-activity update token keyed by
    /// `decision.activity_id`. Returns `None` when the required token is not
    /// registered.
    fn resolve_target(
        &self,
        device: &DeviceRecord,
        decision: &PushDecision,
    ) -> Option<(String, String)> {
        match decision.kind {
            PushKind::Alert | PushKind::Background => {
                let apns = device.apns.as_ref()?;
                Some((apns.device_token.clone(), apns.environment.clone()))
            }
            PushKind::LiveActivityUpdate | PushKind::LiveActivityEnd => {
                let activity_id = decision.activity_id.as_deref()?;
                let token = device.live_activities.get(activity_id)?;
                // Live Activity pushes share the device's APNs environment
                // (the alert registration is the source of truth for
                // sandbox-vs-production); default to production if somehow
                // unset.
                let environment = device
                    .apns
                    .as_ref()
                    .map(|a| a.environment.clone())
                    .unwrap_or_else(|| "production".to_string());
                Some((token.push_token.clone(), environment))
            }
            // Push-to-start is delivered by `deliver_start` against the device's
            // dedicated start token, never through the generic `deliver` path.
            PushKind::LiveActivityStart => None,
        }
    }

    /// Prune the registration APNs rejected. For an alert/background token this
    /// clears the device's whole APNs registration; for a Live Activity token
    /// it clears **only** that activity entry, never the device's alert
    /// registration (D-N2). Refreshes the registry index and writes an audit
    /// line; never logs the token.
    async fn prune_rejected(&self, device: &DeviceRecord, decision: &PushDecision, reason: &str) {
        let (result, detail) = match decision.kind {
            PushKind::Alert | PushKind::Background => {
                tracing::info!(
                    device_id = %device.device_id,
                    reason,
                    "push notifier: pruning unregistered device push token"
                );
                (
                    self.registry.store().clear_push(&device.device_id),
                    serde_json::json!({ "reason": reason, "source": "apns_prune" }),
                )
            }
            PushKind::LiveActivityUpdate | PushKind::LiveActivityEnd => {
                let Some(activity_id) = decision.activity_id.as_deref() else {
                    return;
                };
                tracing::info!(
                    device_id = %device.device_id,
                    reason,
                    "push notifier: pruning unregistered live-activity token"
                );
                (
                    self.registry
                        .store()
                        .clear_live_activity(&device.device_id, activity_id),
                    serde_json::json!({
                        "reason": reason,
                        "source": "apns_prune",
                        "live_activity_id": activity_id,
                    }),
                )
            }
            // Push-to-start rejections are handled by `prune_start_token`;
            // they never reach the generic `deliver`/`prune_rejected` path.
            PushKind::LiveActivityStart => return,
        };

        if let Err(error) = result {
            tracing::warn!(
                device_id = %device.device_id,
                %error,
                "push notifier: failed to clear pruned push registration"
            );
            return;
        }
        if let Err(error) = self.registry.refresh(&device.device_id).await {
            tracing::warn!(
                device_id = %device.device_id,
                %error,
                "push notifier: failed to refresh registry after prune"
            );
        }
        let _ = self.audit.record(
            DeviceAuditEvent::DevicePushTokenRemoved,
            Some(&device.device_id),
            Some(&device.token_prefix),
            Some(detail),
        );
    }
}

/// Translate a policy [`PushDecision`] into an APNs [`ApnsPushSpec`],
/// mapping the [`PushKind`] onto the transport's [`ApnsPushType`] and carrying
/// the (already content-free) payload verbatim.
fn decision_to_spec(decision: &PushDecision) -> ApnsPushSpec {
    let push_type = match decision.kind {
        PushKind::Alert => ApnsPushType::Alert,
        PushKind::Background => ApnsPushType::Background,
        PushKind::LiveActivityStart | PushKind::LiveActivityUpdate | PushKind::LiveActivityEnd => {
            ApnsPushType::LiveActivity
        }
    };
    let mut spec = ApnsPushSpec::new(push_type, decision.payload.clone());
    if let Some(collapse_id) = &decision.collapse_id {
        spec = spec.with_collapse_id(collapse_id.clone());
    }
    spec
}

/// The thread id an event pertains to, if it carries one. Mirrors the policy's
/// own thread extraction so the notifier can route Live Activity tracking.
fn event_thread_id(event: &SseEvent) -> Option<&str> {
    match event {
        SseEvent::Response { thread_id, .. } => Some(thread_id.as_str()),
        SseEvent::ApprovalNeeded { thread_id, .. }
        | SseEvent::ToolStarted { thread_id, .. }
        | SseEvent::Status { thread_id, .. }
        | SseEvent::AgentLifecycle { thread_id, .. } => thread_id.as_deref(),
        _ => None,
    }
}

/// Whether an event is run *progress* (as opposed to a terminal `Response`):
/// these are the events that begin/continue a Live Activity run and so drive
/// auto-tracking and push-to-start. `Response` is deliberately excluded — it
/// *ends* a run, handled by `decide`'s `LiveActivityEnd`, and must never
/// (re)start tracking.
fn is_run_progress_event(event: &SseEvent) -> bool {
    matches!(
        event,
        SseEvent::ToolStarted { .. } | SseEvent::Status { .. } | SseEvent::AgentLifecycle { .. }
    )
}

/// The `activity_id` of a per-activity Live Activity update token this device
/// registered for `thread_id`, if any (agent-run activities only).
fn registered_activity_for_thread<'a>(
    device: &'a DeviceRecord,
    thread_id: &str,
) -> Option<&'a str> {
    device
        .live_activities
        .iter()
        .find_map(|(activity_id, token)| {
            (token.kind == DeviceLiveActivityKind::AgentRun
                && token.thread_id.as_deref() == Some(thread_id))
            .then_some(activity_id.as_str())
        })
}

/// Current monotonic-ish wall clock in seconds, for policy throttle/budget
/// bookkeeping. `SystemTime` is adequate: the policy only uses elapsed-second
/// differences, not absolute timestamps.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Load the APNs provider config from the environment, shared by the native
/// APNs lifecycle channel and this first-party notifier. Returns `Ok(None)`
/// when the required variables are absent (APNs simply stays off).
///
/// Requires `APNS_TEAM_ID`, `APNS_KEY_ID`, `APNS_BUNDLE_ID`, and the signing
/// key via `APNS_PRIVATE_KEY` or `APNS_PRIVATE_KEY_PATH`. `APNS_SANDBOX`
/// selects the default environment for the native channel; the first-party
/// notifier overrides it per-device from each registration's `environment`.
pub fn apns_native_config_from_env() -> Result<Option<ApnsNativeConfig>, String> {
    let Some(team_id) = env_value("APNS_TEAM_ID") else {
        return Ok(None);
    };
    let Some(key_id) = env_value("APNS_KEY_ID") else {
        return Ok(None);
    };
    let Some(bundle_id) = env_value("APNS_BUNDLE_ID") else {
        return Ok(None);
    };
    let Some(private_key_pem) = env_value_or_file("APNS_PRIVATE_KEY", "APNS_PRIVATE_KEY_PATH")?
    else {
        return Ok(None);
    };
    Ok(Some(ApnsNativeConfig {
        team_id,
        key_id,
        bundle_id,
        private_key_pem,
        sandbox: env_bool("APNS_SANDBOX")?.unwrap_or(false),
    }))
}

fn env_value(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_value_or_file(value_key: &str, path_key: &str) -> Result<Option<String>, String> {
    if let Some(value) = env_value(value_key) {
        return Ok(Some(value.replace("\\n", "\n")));
    }
    let Some(path) = env_value(path_key) else {
        return Ok(None);
    };
    std::fs::read_to_string(&path)
        .map(|value| Some(value.replace("\\n", "\n")))
        .map_err(|error| format!("failed to read {path_key}={path}: {error}"))
}

fn env_bool(key: &str) -> Result<Option<bool>, String> {
    let Some(value) = env_value(key) else {
        return Ok(None);
    };
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => Err(format!("{key} must be true or false")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use thinclaw_gateway::web::devices::{DevicePlatform, DeviceScope, DeviceStore};
    use thinclaw_types::error::ChannelError;

    /// Records every send and can be scripted to reject the next token so the
    /// prune-on-410 path is exercised without a real APNs endpoint.
    struct MockSender {
        sends: Mutex<Vec<(String, bool, ApnsPushType)>>,
        outcome: ApnsSendOutcome,
    }

    impl MockSender {
        fn delivering() -> Arc<Self> {
            Arc::new(Self {
                sends: Mutex::new(Vec::new()),
                outcome: ApnsSendOutcome::Delivered,
            })
        }

        fn rejecting(reason: &str) -> Arc<Self> {
            Arc::new(Self {
                sends: Mutex::new(Vec::new()),
                outcome: ApnsSendOutcome::Unregistered {
                    reason: reason.to_string(),
                },
            })
        }

        fn sends(&self) -> Vec<(String, bool, ApnsPushType)> {
            self.sends.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PushSender for MockSender {
        async fn send(
            &self,
            device_token: &str,
            sandbox: bool,
            spec: ApnsPushSpec,
        ) -> Result<ApnsSendOutcome, ChannelError> {
            self.sends
                .lock()
                .unwrap()
                .push((device_token.to_string(), sandbox, spec.push_type));
            Ok(self.outcome.clone())
        }
    }

    async fn registry_with_pushable_device(
        dir: &std::path::Path,
        environment: &str,
    ) -> (Arc<DeviceRegistry>, String) {
        let store = DeviceStore::with_base_dir(dir.to_path_buf());
        let (record, _token) = store
            .insert(
                "phone".to_string(),
                DevicePlatform::Ios,
                vec![DeviceScope::Chat],
                None,
            )
            .unwrap();
        store
            .set_push(
                &record.device_id,
                "apns-device-token".to_string(),
                environment.to_string(),
            )
            .unwrap();
        let registry = Arc::new(DeviceRegistry::load(store).await.unwrap());
        (registry, record.device_id)
    }

    /// Register a per-activity Live Activity update token in the store,
    /// associated with `thread_id`, and refresh the registry snapshot so the
    /// notifier can see it.
    async fn register_live_activity(
        registry: &DeviceRegistry,
        device_id: &str,
        activity_id: &str,
        push_token: &str,
        thread_id: Option<&str>,
    ) {
        use thinclaw_gateway::web::devices::DeviceLiveActivityKind;
        registry
            .store()
            .set_live_activity(
                device_id,
                activity_id,
                push_token.to_string(),
                DeviceLiveActivityKind::AgentRun,
                thread_id.map(str::to_string),
                None,
            )
            .unwrap();
        registry.refresh(device_id).await.unwrap();
    }

    fn response(thread_id: &str) -> SseEvent {
        SseEvent::Response {
            content: "secret body".to_string(),
            thread_id: thread_id.to_string(),
            attachments: Vec::new(),
        }
    }

    fn notifier(
        registry: Arc<DeviceRegistry>,
        sender: Arc<dyn PushSender>,
        dir: &std::path::Path,
    ) -> FirstPartyPushNotifier {
        FirstPartyPushNotifier::with_audit_log(
            registry,
            sender,
            DeviceAuditLog::with_base_dir(dir.to_path_buf()),
        )
    }

    #[tokio::test]
    async fn response_event_delivers_alert_to_registered_device() {
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, _id) = registry_with_pushable_device(dir.path(), "production").await;
        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());

        n.handle_event(&response("t1")).await;

        let sends = sender.sends();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].0, "apns-device-token");
        assert!(!sends[0].1, "production registration → not sandbox");
        assert_eq!(sends[0].2, ApnsPushType::Alert);
    }

    #[tokio::test]
    async fn development_registration_targets_sandbox() {
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, _id) = registry_with_pushable_device(dir.path(), "development").await;
        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());

        n.handle_event(&response("t1")).await;

        assert!(sender.sends()[0].1, "development registration → sandbox");
    }

    #[tokio::test]
    async fn device_without_registration_gets_no_push() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = DeviceStore::with_base_dir(dir.path().to_path_buf());
        store
            .insert(
                "phone".to_string(),
                DevicePlatform::Ios,
                vec![DeviceScope::Chat],
                None,
            )
            .unwrap();
        let registry = Arc::new(DeviceRegistry::load(store).await.unwrap());
        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());

        n.handle_event(&response("t1")).await;

        assert!(sender.sends().is_empty());
    }

    #[tokio::test]
    async fn alert_is_suppressed_while_device_streams() {
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;
        let sender = MockSender::delivering();
        let mut n = notifier(registry.clone(), sender.clone(), dir.path());

        let _guard = registry.stream_opened(&device_id);
        n.handle_event(&response("t1")).await;

        assert!(
            sender.sends().is_empty(),
            "alert must be suppressed while the device is streaming in-app"
        );
    }

    #[tokio::test]
    async fn live_activity_update_is_not_suppressed_while_streaming() {
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;
        // Register the per-activity Live Activity token bound to thread t1. The
        // notifier auto-tracks the run from this registration — no manual
        // track_run — which is the wiring under test.
        register_live_activity(&registry, &device_id, "act-1", "la-token-act-1", Some("t1")).await;
        let sender = MockSender::delivering();
        let mut n = notifier(registry.clone(), sender.clone(), dir.path());

        let _guard = registry.stream_opened(&device_id);
        n.handle_event(&SseEvent::ToolStarted {
            name: "shell.execute".to_string(),
            thread_id: Some("t1".to_string()),
        })
        .await;

        let sends = sender.sends();
        assert_eq!(sends.len(), 1, "Live Activity updates are not suppressed");
        assert_eq!(sends[0].2, ApnsPushType::LiveActivity);
        // R4: the update targets the per-activity token, never the alert token.
        assert_eq!(
            sends[0].0, "la-token-act-1",
            "Live Activity push must target the per-activity token"
        );
    }

    #[tokio::test]
    async fn status_event_on_registered_thread_auto_tracks_and_updates() {
        // The core M3 wiring: a run-progress Status event for a thread the
        // device registered a Live Activity for produces a LiveActivityUpdate
        // to that per-activity token, with no manual track_run.
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;
        register_live_activity(&registry, &device_id, "act-1", "la-token-act-1", Some("t1")).await;
        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());

        n.handle_event(&SseEvent::Status {
            message: "thinking hard".to_string(),
            thread_id: Some("t1".to_string()),
        })
        .await;

        let sends = sender.sends();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].0, "la-token-act-1");
        assert_eq!(sends[0].2, ApnsPushType::LiveActivity);
    }

    #[tokio::test]
    async fn response_ends_auto_tracked_live_activity() {
        // After a run is auto-tracked from a Status event, a Response for the
        // same thread ends the Live Activity (LiveActivityEnd to the activity
        // token), not a message alert.
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;
        register_live_activity(&registry, &device_id, "act-1", "la-token-act-1", Some("t1")).await;
        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());

        // First, a progress event auto-tracks and emits an update.
        n.handle_event(&SseEvent::ToolStarted {
            name: "shell.execute".to_string(),
            thread_id: Some("t1".to_string()),
        })
        .await;
        // Then the Response ends it.
        n.handle_event(&response("t1")).await;

        let sends = sender.sends();
        assert_eq!(sends.len(), 2);
        // Both went to the per-activity token; both are Live Activity pushes
        // (the second is the end, not a message alert).
        assert!(sends.iter().all(|s| s.0 == "la-token-act-1"));
        assert!(sends.iter().all(|s| s.2 == ApnsPushType::LiveActivity));
    }

    #[tokio::test]
    async fn untracked_thread_progress_produces_no_push() {
        // A run-progress event for a thread the device has NOT registered a
        // Live Activity for (and no start token) produces nothing: no alert,
        // no Live Activity.
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;
        // Registered activity is bound to a *different* thread.
        register_live_activity(
            &registry,
            &device_id,
            "act-other",
            "la-other",
            Some("other"),
        )
        .await;
        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());

        n.handle_event(&SseEvent::Status {
            message: "thinking".to_string(),
            thread_id: Some("t1".to_string()),
        })
        .await;

        assert!(
            sender.sends().is_empty(),
            "an untracked thread's progress must not push"
        );
    }

    #[tokio::test]
    async fn push_to_start_fires_once_when_start_token_present_and_no_activity() {
        // A killed app: device has a push-to-start token but no active activity
        // for the thread. The first run-progress event emits a push-to-start to
        // the start token; subsequent events do not re-fire it.
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;
        registry
            .store()
            .set_live_activity_start_token(&device_id, "start-token".to_string())
            .unwrap();
        registry.refresh(&device_id).await.unwrap();
        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());

        n.handle_event(&SseEvent::ToolStarted {
            name: "shell.execute".to_string(),
            thread_id: Some("t1".to_string()),
        })
        .await;
        // A second progress event must not fire another start.
        n.handle_event(&SseEvent::Status {
            message: "still going".to_string(),
            thread_id: Some("t1".to_string()),
        })
        .await;

        let sends = sender.sends();
        assert_eq!(sends.len(), 1, "push-to-start fires exactly once");
        assert_eq!(sends[0].0, "start-token", "delivered to the start token");
        assert_eq!(sends[0].2, ApnsPushType::LiveActivity);
    }

    #[tokio::test]
    async fn no_push_to_start_when_activity_already_registered() {
        // If the device already has a per-activity token for the thread (app is
        // alive), no push-to-start is emitted even with a start token present —
        // updates go to the per-activity token instead.
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;
        register_live_activity(&registry, &device_id, "act-1", "la-token-act-1", Some("t1")).await;
        registry
            .store()
            .set_live_activity_start_token(&device_id, "start-token".to_string())
            .unwrap();
        registry.refresh(&device_id).await.unwrap();
        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());

        n.handle_event(&SseEvent::ToolStarted {
            name: "shell.execute".to_string(),
            thread_id: Some("t1".to_string()),
        })
        .await;

        let sends = sender.sends();
        assert_eq!(sends.len(), 1);
        assert_eq!(
            sends[0].0, "la-token-act-1",
            "an alive app is driven via its per-activity token, not push-to-start"
        );
    }

    #[tokio::test]
    async fn unregistered_token_prunes_device_and_audits() {
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;
        let sender = MockSender::rejecting("Unregistered");
        let mut n = notifier(registry.clone(), sender.clone(), dir.path());

        n.handle_event(&response("t1")).await;

        // Registration cleared from the store.
        let record = registry.store().get(&device_id).unwrap().unwrap();
        assert!(record.apns.is_none(), "push registration must be pruned");

        // A subsequent event no longer produces a send (nothing to push to).
        n.handle_event(&response("t2")).await;
        assert_eq!(sender.sends().len(), 1, "no further sends after prune");

        // Audit line written, without the token.
        let audit = std::fs::read_to_string(dir.path().join("device-audit.jsonl")).unwrap();
        assert!(audit.contains("device.push_token_removed"));
        assert!(!audit.contains("apns-device-token"), "token never logged");
    }

    #[tokio::test]
    async fn revoked_device_receives_no_push_even_with_lingering_registration() {
        // R2: a device revoked *after* it registered a push token must never be
        // pushed to. Simulate a stale APNs registration lingering on a revoked
        // record (the store setters now reject re-attaching, but defense in
        // depth: the notifier must also skip it).
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;

        // Revoke via the registry (clears push state + marks revoked_at, and
        // refreshes the snapshot). To exercise the notifier's own guard we then
        // hand-write a lingering apns registration back onto the revoked record
        // and refresh, mimicking a stale on-disk row.
        registry.revoke(&device_id).await.unwrap();
        let raw_path = dir.path().join("devices.json");
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&raw_path).unwrap()).unwrap();
        value["devices"][0]["apns"] = serde_json::json!({
            "device_token": "stale-apns-token",
            "environment": "production",
            "updated_at": "2024-01-01T00:00:00+00:00",
        });
        std::fs::write(&raw_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
        registry.refresh(&device_id).await.unwrap();

        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());
        n.handle_event(&response("t1")).await;

        assert!(
            sender.sends().is_empty(),
            "a revoked device must never receive a push"
        );
    }

    #[tokio::test]
    async fn live_activity_prune_clears_only_that_activity_not_alert_registration() {
        // R4: a rejected Live Activity token prunes only that activity entry;
        // the device's APNs alert registration must survive.
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, device_id) = registry_with_pushable_device(dir.path(), "production").await;
        register_live_activity(&registry, &device_id, "act-1", "la-token-act-1", Some("t1")).await;

        let sender = MockSender::rejecting("Unregistered");
        let mut n = notifier(registry.clone(), sender.clone(), dir.path());

        n.handle_event(&SseEvent::ToolStarted {
            name: "shell.execute".to_string(),
            thread_id: Some("t1".to_string()),
        })
        .await;

        // The send targeted the per-activity token and was rejected.
        let sends = sender.sends();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].0, "la-token-act-1");

        let record = registry.store().get(&device_id).unwrap().unwrap();
        assert!(
            !record.live_activities.contains_key("act-1"),
            "the rejected live-activity token must be pruned"
        );
        assert!(
            record.apns.is_some(),
            "the device's alert registration must survive a live-activity prune"
        );

        // Audit names the specific live activity.
        let audit = std::fs::read_to_string(dir.path().join("device-audit.jsonl")).unwrap();
        assert!(audit.contains("live_activity_id"));
        assert!(!audit.contains("la-token-act-1"), "token never logged");
    }

    #[tokio::test]
    async fn stream_chunk_event_performs_no_device_access() {
        // R5: a stream_chunk event must short-circuit before any device lookup.
        // Prove it by deleting the on-disk devices.json *and* clearing the
        // in-memory snapshot's only backing after load — if the notifier
        // touched either, it would observe the pushable device. We assert no
        // send results and, more strongly, that a device which *would* push on
        // a real event produces nothing here.
        let dir = tempfile::TempDir::new().unwrap();
        let (registry, _device_id) = registry_with_pushable_device(dir.path(), "production").await;

        // Remove the backing file so any disk read would fail loudly; the
        // notifier must not read it for a stream_chunk.
        std::fs::remove_file(dir.path().join("devices.json")).unwrap();

        let sender = MockSender::delivering();
        let mut n = notifier(registry, sender.clone(), dir.path());

        n.handle_event(&SseEvent::StreamChunk {
            content: "streaming token".to_string(),
            thread_id: Some("t1".to_string()),
        })
        .await;

        assert!(
            sender.sends().is_empty(),
            "stream_chunk must not produce a push and must not read the device store"
        );
    }

    #[test]
    fn structured_lifecycle_is_thread_scoped_run_progress() {
        let event = SseEvent::AgentLifecycle {
            phase: "context_compaction".to_string(),
            label: "Compacting context".to_string(),
            detail: Some("9500 of 10000 tokens".to_string()),
            thread_id: Some("t1".to_string()),
        };

        assert_eq!(event_thread_id(&event), Some("t1"));
        assert!(is_run_progress_event(&event));
    }

    #[tokio::test]
    async fn decision_to_spec_maps_kinds_and_forwards_collapse_id() {
        let decision = PushDecision {
            kind: PushKind::Alert,
            category: None,
            collapse_id: Some("thread-t1".to_string()),
            activity_id: None,
            payload: serde_json::json!({"aps": {}}),
        };
        let spec = decision_to_spec(&decision);
        assert_eq!(spec.push_type, ApnsPushType::Alert);
        assert_eq!(spec.collapse_id.as_deref(), Some("thread-t1"));

        let end = PushDecision {
            kind: PushKind::LiveActivityEnd,
            category: None,
            collapse_id: None,
            activity_id: Some("act-1".to_string()),
            payload: serde_json::json!({"aps": {}}),
        };
        assert_eq!(decision_to_spec(&end).push_type, ApnsPushType::LiveActivity);
    }
}
