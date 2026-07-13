//! Pure push-notification policy: map an [`SseEvent`] plus per-device state
//! to an optional content-free APNs [`PushDecision`].
//!
//! This module is deliberately I/O-free and dependency-light so it can be
//! unit-tested exhaustively. It does *not* build APNs request headers or talk
//! to Apple — the runtime notifier (`src/channels/first_party_push.rs`) owns
//! that, translating a [`PushDecision`] into a `thinclaw_channels::ApnsPushSpec`
//! and delivering it. The split keeps the privacy-critical payload shaping
//! (D-N1/D-N2) in one auditable place with no network in the way.
//!
//! ## Content-free contract (`docs/MOBILE_SECURITY.md` D-N1/D-N2)
//!
//! - Alert payloads carry a *generic* `aps.alert` (`{"title":"ThinClaw",
//!   "body":"New activity"}`), `mutable-content: 1`, and a category. The
//!   custom `tc` dict carries **only ids** (`thread_id`, `request_id`,
//!   `job_id`) — never message text, tool names, or parameters. A Notification
//!   Service Extension fetches real content over the pinned connection and
//!   rewrites locally.
//! - Live Activity `content-state` carries **only** `{phase, progress?,
//!   revision}` (matching `apps/ios` `AgentRunAttributes.ContentState`) — no
//!   prompt text, no tool arguments, not even the tool name.
//!
//! ## Mapping (see `docs/MOBILE_APP.md`)
//!
//! | Event | Decision |
//! |---|---|
//! | `Response` | `Alert` `THINCLAW_MESSAGE`, collapse `thread-{id}`; closes a tracked run → also `LiveActivityEnd` |
//! | `ApprovalNeeded` | `Alert` `THINCLAW_APPROVAL` (time-sensitive) |
//! | `JobResult` | `Alert` `THINCLAW_JOB` |
//! | `ToolStarted`/`Status` while a run is tracked | throttled `LiveActivityUpdate` (min 15 s/activity, monotonic revision) |
//! | background wake | under a per-device token bucket (3/hour) |

use std::collections::HashMap;

use super::approval_risk::ApprovalRisk;
use crate::web::types::SseEvent;

/// Minimum interval between Live Activity update pushes for a single activity
/// (D-N2 throttle). Runtime status churn must not fan out to APNs faster than
/// this per activity.
pub const LIVE_ACTIVITY_MIN_INTERVAL_SECS: u64 = 15;

/// Per-device background-wake budget: how many silent `background` pushes may
/// be sent per rolling hour before the bucket is empty.
pub const BACKGROUND_WAKE_BUDGET: u32 = 3;

/// Window over which [`BACKGROUND_WAKE_BUDGET`] refills.
pub const BACKGROUND_WAKE_WINDOW_SECS: u64 = 3600;

/// The generic alert title shown before the Notification Service Extension
/// rewrites it locally. Never contains message content.
pub const GENERIC_ALERT_TITLE: &str = "ThinClaw";
/// The generic alert body shown before local rewrite. Never contains message
/// content.
pub const GENERIC_ALERT_BODY: &str = "New activity";

/// APNs categories (`aps.category`), used by the client to route/render.
pub const CATEGORY_MESSAGE: &str = "THINCLAW_MESSAGE";
/// Back-compat base approval category. Retained for callers that route on the
/// approval family; live approval pushes use the risk-split categories below so
/// the client can offer interactive approve-from-notification for low-risk
/// approvals only (D-N3) and gate high-risk ones behind Face ID in-app (D-K3).
pub const CATEGORY_APPROVAL: &str = "THINCLAW_APPROVAL";
/// Low-risk approval category: interactive approve-from-notification allowed.
pub const CATEGORY_APPROVAL_LOW: &str = "THINCLAW_APPROVAL_LOW";
/// High-risk approval category: no interactive action; deep-link into the app
/// for a Face ID-gated approval.
pub const CATEGORY_APPROVAL_HIGH: &str = "THINCLAW_APPROVAL_HIGH";
pub const CATEGORY_JOB: &str = "THINCLAW_JOB";

/// The push category (`apns-push-type`) a [`PushDecision`] targets. The
/// runtime notifier maps these onto `thinclaw_channels::ApnsPushType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushKind {
    /// User-visible alert (`apns-push-type: alert`).
    Alert,
    /// Silent content-available wake (`apns-push-type: background`), spent
    /// against the per-device wake budget.
    Background,
    /// Live Activity push-to-start (`apns-push-type: liveactivity`, `event:
    /// start`): spawns a fresh activity on a device that has no active activity
    /// for the run. Delivered to the device's push-to-start token, not a
    /// per-activity update token.
    LiveActivityStart,
    /// Live Activity update (`apns-push-type: liveactivity`).
    LiveActivityUpdate,
    /// Final Live Activity update that dismisses the activity
    /// (`content-state` with a terminal phase, `event: end`).
    LiveActivityEnd,
}

/// A content-free push to deliver to one device.
///
/// `payload` is the full APNs JSON body (the `aps` dict plus, for alerts, a
/// `tc` id-only dict). It never contains message text, tool names, or
/// parameters — see the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct PushDecision {
    pub kind: PushKind,
    /// `aps.category` for alerts; `None` for Live Activity / background pushes.
    pub category: Option<&'static str>,
    /// `apns-collapse-id`, used to coalesce (e.g. `thread-{id}`,
    /// `run-{activity_id}`).
    pub collapse_id: Option<String>,
    /// For [`PushKind::LiveActivityUpdate`]/[`PushKind::LiveActivityEnd`], the
    /// APNs `activity_id` this push targets. The runtime notifier uses it to
    /// look up the device's *per-activity* update token
    /// (`live_activities[activity_id].push_token`) rather than the device's
    /// alert token, and to prune only that activity on a token rejection.
    /// `None` for alert/background decisions.
    pub activity_id: Option<String>,
    /// The content-free APNs payload.
    pub payload: serde_json::Value,
}

/// A Live Activity run this device is tracking, keyed by its thread id.
#[derive(Debug, Clone)]
struct TrackedRun {
    /// APNs `activity_id` (the collapse/coalesce key for its updates).
    activity_id: String,
    /// Monotonic revision counter — every emitted update/end increments it so
    /// a late push never regresses UI driven by local SSE updates.
    revision: u64,
    /// Monotonic clock (seconds) of the last emitted update, for throttling.
    last_update_secs: Option<u64>,
}

/// Per-device policy state. Owned and mutated by the runtime notifier; the
/// policy functions take `&mut` so throttle/revision/budget bookkeeping is
/// updated atomically with the decision that consumes it.
///
/// This is intentionally *not* persisted: it is soft runtime state (throttle
/// windows, wake budget, in-flight run revisions). A restart simply starts the
/// budget full and revisions at zero, which is safe.
#[derive(Debug, Default)]
pub struct DevicePushState {
    /// Live runs this device is tracking, keyed by `thread_id`.
    tracked_runs: HashMap<String, TrackedRun>,
    /// Monotonic-clock timestamps (seconds) of recent background wakes, oldest
    /// first, pruned to the rolling window on each check.
    wake_events: Vec<u64>,
    /// Threads for which a Live Activity push-to-start has already been emitted
    /// this session, so a killed app is only asked to spawn one activity per
    /// run. Cleared for a thread when its run ends (the activity is dismissed).
    started_threads: std::collections::HashSet<String>,
}

impl DevicePushState {
    /// Begin tracking a Live Activity run for `thread_id` under `activity_id`,
    /// resetting revision/throttle state. The runtime notifier calls this when
    /// a device registers a Live Activity token (D-N2) so subsequent
    /// run-progress events can drive updates.
    pub fn track_run(&mut self, thread_id: impl Into<String>, activity_id: impl Into<String>) {
        let thread_id = thread_id.into();
        self.tracked_runs.insert(
            thread_id,
            TrackedRun {
                activity_id: activity_id.into(),
                revision: 0,
                last_update_secs: None,
            },
        );
    }

    /// Ensure a run is tracked for `thread_id` under `activity_id` without
    /// disturbing the monotonic revision/throttle state of an existing track.
    ///
    /// Returns `true` when this call *started* tracking (i.e. the run was not
    /// already tracked), which the notifier treats as "the run began" for
    /// push-to-start purposes. If a run is already tracked under a *different*
    /// `activity_id` (e.g. a push-to-start placeholder replaced by the real
    /// activity the app registered), the id is updated in place and revision is
    /// preserved so late pushes never regress the UI.
    pub fn ensure_tracked(
        &mut self,
        thread_id: impl Into<String>,
        activity_id: impl Into<String>,
    ) -> bool {
        let thread_id = thread_id.into();
        let activity_id = activity_id.into();
        match self.tracked_runs.get_mut(&thread_id) {
            Some(run) => {
                if run.activity_id != activity_id {
                    run.activity_id = activity_id;
                }
                false
            }
            None => {
                self.tracked_runs.insert(
                    thread_id,
                    TrackedRun {
                        activity_id,
                        revision: 0,
                        last_update_secs: None,
                    },
                );
                true
            }
        }
    }

    /// Stop tracking the run for `thread_id`, if any. Also clears the
    /// push-to-start fire-once marker so a genuinely new run later can start a
    /// fresh activity.
    pub fn untrack_run(&mut self, thread_id: &str) {
        self.tracked_runs.remove(thread_id);
        self.started_threads.remove(thread_id);
    }

    /// True if a Live Activity run is currently tracked for `thread_id`.
    pub fn is_tracking(&self, thread_id: &str) -> bool {
        self.tracked_runs.contains_key(thread_id)
    }

    /// The `activity_id` currently tracked for `thread_id`, if any.
    pub fn tracked_activity_id(&self, thread_id: &str) -> Option<&str> {
        self.tracked_runs
            .get(thread_id)
            .map(|run| run.activity_id.as_str())
    }

    /// Record that a push-to-start has been emitted for `thread_id`, so it is
    /// only sent once per run. Returns `true` if this is the first time (caller
    /// should emit the start), `false` if it already fired.
    pub fn mark_start_emitted(&mut self, thread_id: impl Into<String>) -> bool {
        self.started_threads.insert(thread_id.into())
    }

    /// Remaining background-wake budget at `now_secs` (after pruning expired
    /// events). Exposed for tests/telemetry.
    pub fn wake_budget_remaining(&mut self, now_secs: u64) -> u32 {
        self.prune_wakes(now_secs);
        BACKGROUND_WAKE_BUDGET.saturating_sub(self.wake_events.len() as u32)
    }

    fn prune_wakes(&mut self, now_secs: u64) {
        // Keep events still inside the rolling window. Using elapsed-time
        // comparison (rather than an absolute cutoff) avoids a boundary
        // artifact when `now_secs` is small enough that `now - window`
        // saturates to 0.
        self.wake_events
            .retain(|&t| now_secs.saturating_sub(t) < BACKGROUND_WAKE_WINDOW_SECS);
    }

    /// Try to spend one wake from the budget at `now_secs`. Returns `true` if
    /// a wake was available and consumed; `false` if the bucket is empty.
    fn try_spend_wake(&mut self, now_secs: u64) -> bool {
        self.prune_wakes(now_secs);
        if (self.wake_events.len() as u32) < BACKGROUND_WAKE_BUDGET {
            self.wake_events.push(now_secs);
            true
        } else {
            false
        }
    }
}

/// Extract the `thread_id` an event pertains to, if it carries one.
fn event_thread_id(event: &SseEvent) -> Option<&str> {
    match event {
        SseEvent::Response { thread_id, .. } => Some(thread_id.as_str()),
        SseEvent::ApprovalNeeded { thread_id, .. }
        | SseEvent::ToolStarted { thread_id, .. }
        | SseEvent::Status { thread_id, .. }
        | SseEvent::ContextPressure { thread_id, .. }
        | SseEvent::AgentLifecycle { thread_id, .. } => thread_id.as_deref(),
        _ => None,
    }
}

/// The generic, content-free `aps.alert` object.
fn generic_alert() -> serde_json::Value {
    serde_json::json!({
        "title": GENERIC_ALERT_TITLE,
        "body": GENERIC_ALERT_BODY,
    })
}

/// Build a content-free alert payload: generic alert, `mutable-content: 1`,
/// the category, an interruption level, and a `tc` id-only dict. `tc` carries
/// only the provided ids — never text.
fn alert_payload(
    category: &str,
    interruption_level: Option<&str>,
    tc: serde_json::Value,
) -> serde_json::Value {
    let mut aps = serde_json::json!({
        "alert": generic_alert(),
        "mutable-content": 1,
        "category": category,
    });
    if let Some(level) = interruption_level {
        aps["interruption-level"] = serde_json::Value::String(level.to_string());
    }
    serde_json::json!({ "aps": aps, "tc": tc })
}

/// The `content-state` for a Live Activity push: `{phase, progress?,
/// revision}` only, matching `apps/ios` `AgentRunAttributes.ContentState`. No
/// tool name, no text.
fn content_state(phase: &str, progress: Option<i64>, revision: u64) -> serde_json::Value {
    let mut state = serde_json::json!({
        "phase": phase,
        "revision": revision,
    });
    if let Some(progress) = progress {
        state["progress"] = serde_json::json!(progress);
    }
    state
}

/// Build a Live Activity payload with the given `event` (`update`/`end`) and
/// content-state. `apps/ios` reads `aps.content-state`; `event` and
/// `timestamp` are the ActivityKit envelope fields.
fn live_activity_payload(
    event: &str,
    now_secs: u64,
    phase: &str,
    progress: Option<i64>,
    revision: u64,
) -> serde_json::Value {
    #[allow(clippy::cast_possible_wrap)]
    let timestamp = now_secs as i64;
    serde_json::json!({
        "aps": {
            "timestamp": timestamp,
            "event": event,
            "content-state": content_state(phase, progress, revision),
        }
    })
}

/// Whether this event *kind* could ever produce a push for some device, before
/// any per-device state is consulted. The runtime notifier calls this first so
/// the overwhelmingly common streaming events (chunks, thinking, heartbeats)
/// short-circuit **before** any device lookup or snapshot.
///
/// This must stay in sync with the arms of [`decide`]: every event variant
/// `decide` can map to `Some(..)` must return `true` here. Returning `true`
/// for an event that ultimately maps to `None` is safe (just does the normal
/// per-device scan); returning `false` for one that can push is a bug.
pub fn can_produce_push(event: &SseEvent) -> bool {
    matches!(
        event,
        SseEvent::Response { .. }
            | SseEvent::ApprovalNeeded { .. }
            | SseEvent::JobResult { .. }
            | SseEvent::JobSessionResult { .. }
            | SseEvent::ToolStarted { .. }
            | SseEvent::Status { .. }
            | SseEvent::ContextPressure { .. }
            | SseEvent::AgentLifecycle { .. }
    )
}

/// Decide the push (if any) for `event` given this device's `state` and the
/// current monotonic clock `now_secs`.
///
/// Pure apart from mutating `state` (throttle windows, revision counters, wake
/// budget) so a caller can apply the decision and its bookkeeping atomically.
/// Returns `None` when the event should not produce a push for this device.
pub fn decide(
    event: &SseEvent,
    state: &mut DevicePushState,
    now_secs: u64,
) -> Option<PushDecision> {
    match event {
        // A response closing a tracked run ends its Live Activity; otherwise a
        // collapsible message alert.
        SseEvent::Response { thread_id, .. } => {
            if state.is_tracking(thread_id) {
                Some(end_live_activity(state, thread_id, now_secs, "done"))
            } else {
                Some(PushDecision {
                    kind: PushKind::Alert,
                    category: Some(CATEGORY_MESSAGE),
                    collapse_id: Some(format!("thread-{thread_id}")),
                    activity_id: None,
                    payload: alert_payload(CATEGORY_MESSAGE, None, thread_tc(thread_id)),
                })
            }
        }

        // Approvals are time-sensitive actionable alerts. The category is split
        // by the gateway-computed risk tier (D-K3): low-risk approvals get an
        // interactive-capable category, high-risk approvals a category the
        // client renders without an inline approve action (deep-link to the
        // Face ID-gated approval instead, D-N3).
        SseEvent::ApprovalNeeded {
            request_id,
            thread_id,
            risk,
            ..
        } => {
            let category = match risk {
                ApprovalRisk::Low => CATEGORY_APPROVAL_LOW,
                ApprovalRisk::High => CATEGORY_APPROVAL_HIGH,
            };
            let mut tc = serde_json::json!({ "request_id": request_id });
            if let Some(thread_id) = thread_id {
                tc["thread_id"] = serde_json::Value::String(thread_id.clone());
            }
            Some(PushDecision {
                kind: PushKind::Alert,
                category: Some(category),
                collapse_id: Some(format!("approval-{request_id}")),
                activity_id: None,
                payload: alert_payload(category, Some("time-sensitive"), tc),
            })
        }

        // Terminal job events → job alerts.
        SseEvent::JobResult { job_id, .. } | SseEvent::JobSessionResult { job_id, .. } => {
            Some(PushDecision {
                kind: PushKind::Alert,
                category: Some(CATEGORY_JOB),
                collapse_id: Some(format!("job-{job_id}")),
                activity_id: None,
                payload: alert_payload(CATEGORY_JOB, None, serde_json::json!({ "job_id": job_id })),
            })
        }

        // Run-progress churn drives throttled Live Activity updates, but only
        // while a run is actively tracked for that thread.
        SseEvent::ToolStarted { .. } => {
            let thread_id = event_thread_id(event)?;
            live_activity_update(state, thread_id, now_secs, "runningTool", None)
        }
        SseEvent::Status { .. }
        | SseEvent::ContextPressure { .. }
        | SseEvent::AgentLifecycle { .. } => {
            let thread_id = event_thread_id(event)?;
            live_activity_update(state, thread_id, now_secs, "thinking", None)
        }

        _ => None,
    }
}

/// The id-only `tc` dict for a thread alert.
fn thread_tc(thread_id: &str) -> serde_json::Value {
    serde_json::json!({ "thread_id": thread_id })
}

/// Emit a throttled Live Activity update for a tracked run, or `None` if the
/// run isn't tracked or the throttle window hasn't elapsed. Increments the
/// run's revision only when an update is actually emitted.
fn live_activity_update(
    state: &mut DevicePushState,
    thread_id: &str,
    now_secs: u64,
    phase: &str,
    progress: Option<i64>,
) -> Option<PushDecision> {
    let run = state.tracked_runs.get_mut(thread_id)?;
    if let Some(last) = run.last_update_secs
        && now_secs.saturating_sub(last) < LIVE_ACTIVITY_MIN_INTERVAL_SECS
    {
        return None;
    }
    run.revision += 1;
    run.last_update_secs = Some(now_secs);
    let revision = run.revision;
    let activity_id = run.activity_id.clone();
    Some(PushDecision {
        kind: PushKind::LiveActivityUpdate,
        category: None,
        collapse_id: Some(format!("run-{activity_id}")),
        activity_id: Some(activity_id),
        payload: live_activity_payload("update", now_secs, phase, progress, revision),
    })
}

/// End a tracked run's Live Activity (terminal `phase`), untracking it. The
/// end push is never throttled and always bumps the revision so it wins over
/// any in-flight local update.
fn end_live_activity(
    state: &mut DevicePushState,
    thread_id: &str,
    now_secs: u64,
    phase: &str,
) -> PushDecision {
    let (activity_id, revision) = match state.tracked_runs.get_mut(thread_id) {
        Some(run) => {
            run.revision += 1;
            (run.activity_id.clone(), run.revision)
        }
        None => (thread_id.to_string(), 1),
    };
    state.untrack_run(thread_id);
    PushDecision {
        kind: PushKind::LiveActivityEnd,
        category: None,
        collapse_id: Some(format!("run-{activity_id}")),
        activity_id: Some(activity_id),
        payload: live_activity_payload("end", now_secs, phase, None, revision),
    }
}

/// Build a Live Activity push-to-start decision for a run that is beginning on
/// a device with no active activity (D-N2). The payload is an ActivityKit
/// `event: start` envelope carrying only the content-free initial
/// `content-state` (`{phase, revision}`) plus the (id-only) `attributes` needed
/// to spawn the activity — never prompt text or tool arguments.
///
/// `activity_id` seeds the notifier's tracking so the app's subsequently
/// registered per-activity update token (which carries the same `thread_id`)
/// can take over update delivery. The push is delivered to the device's
/// push-to-start token by the runtime notifier, never a per-activity token.
pub fn live_activity_start(thread_id: &str, activity_id: &str, now_secs: u64) -> PushDecision {
    #[allow(clippy::cast_possible_wrap)]
    let timestamp = now_secs as i64;
    PushDecision {
        kind: PushKind::LiveActivityStart,
        category: None,
        collapse_id: Some(format!("run-{activity_id}")),
        activity_id: Some(activity_id.to_string()),
        payload: serde_json::json!({
            "aps": {
                "timestamp": timestamp,
                "event": "start",
                "content-state": content_state("thinking", None, 0),
                "attributes-type": "AgentRunAttributes",
                "attributes": { "thread_id": thread_id },
            }
        }),
    }
}

/// Decide a background wake push, spending one unit of the device's wake
/// budget. Returns `None` when the budget is exhausted. The `payload` is a
/// silent `content-available` push carrying only the provided id-only `tc`.
///
/// Callers that want a wake (e.g. to nudge the Notification Service Extension
/// to fetch content while the app is backgrounded) use this rather than
/// [`decide`], which never emits background pushes on its own.
pub fn decide_background_wake(
    state: &mut DevicePushState,
    now_secs: u64,
    tc: serde_json::Value,
) -> Option<PushDecision> {
    if !state.try_spend_wake(now_secs) {
        return None;
    }
    Some(PushDecision {
        kind: PushKind::Background,
        category: None,
        collapse_id: None,
        activity_id: None,
        payload: serde_json::json!({
            "aps": { "content-available": 1 },
            "tc": tc,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(thread_id: &str, content: &str) -> SseEvent {
        SseEvent::Response {
            content: content.to_string(),
            thread_id: thread_id.to_string(),
            attachments: Vec::new(),
        }
    }

    fn approval(request_id: &str, thread_id: Option<&str>, tool: &str, params: &str) -> SseEvent {
        SseEvent::ApprovalNeeded {
            request_id: request_id.to_string(),
            tool_name: tool.to_string(),
            description: "please approve".to_string(),
            parameters: params.to_string(),
            risk: super::super::approval_risk::classify(tool, params),
            thread_id: thread_id.map(str::to_string),
        }
    }

    /// Build an approval event with an explicit risk tier, for exercising the
    /// category split independent of the classifier.
    fn approval_with_risk(request_id: &str, risk: ApprovalRisk) -> SseEvent {
        SseEvent::ApprovalNeeded {
            request_id: request_id.to_string(),
            tool_name: "tool".to_string(),
            description: "please approve".to_string(),
            parameters: "{}".to_string(),
            risk,
            thread_id: None,
        }
    }

    fn job_result(job_id: &str, message: &str) -> SseEvent {
        SseEvent::JobResult {
            job_id: job_id.to_string(),
            status: "completed".to_string(),
            session_id: None,
            success: Some(true),
            message: Some(message.to_string()),
        }
    }

    fn tool_started(thread_id: &str, name: &str) -> SseEvent {
        SseEvent::ToolStarted {
            name: name.to_string(),
            thread_id: Some(thread_id.to_string()),
        }
    }

    fn status(thread_id: &str, message: &str) -> SseEvent {
        SseEvent::Status {
            message: message.to_string(),
            thread_id: Some(thread_id.to_string()),
        }
    }

    fn context_pressure(thread_id: &str, level: &str, usage_percent: f64) -> SseEvent {
        SseEvent::ContextPressure {
            level: level.to_string(),
            usage_percent,
            thread_id: Some(thread_id.to_string()),
        }
    }

    /// Serialize `payload` and assert none of the sensitive `needles` appear —
    /// the content-free invariant (D-N1/D-N2).
    fn assert_content_free(payload: &serde_json::Value, needles: &[&str]) {
        let serialized = serde_json::to_string(payload).unwrap();
        for needle in needles {
            assert!(
                !serialized.contains(needle),
                "payload leaked {needle:?}: {serialized}"
            );
        }
    }

    #[test]
    fn response_maps_to_collapsible_message_alert() {
        let mut state = DevicePushState::default();
        let decision = decide(&response("t1", "secret transcript body"), &mut state, 0).unwrap();
        assert_eq!(decision.kind, PushKind::Alert);
        assert_eq!(decision.category, Some(CATEGORY_MESSAGE));
        assert_eq!(decision.collapse_id.as_deref(), Some("thread-t1"));
        assert_content_free(&decision.payload, &["secret transcript body"]);
        // tc carries only the thread id.
        assert_eq!(decision.payload["tc"]["thread_id"], "t1");
        assert_eq!(
            decision.payload["aps"]["alert"]["title"],
            GENERIC_ALERT_TITLE
        );
        assert_eq!(decision.payload["aps"]["mutable-content"], 1);
    }

    #[test]
    fn high_risk_approval_maps_to_time_sensitive_high_category_alert() {
        let mut state = DevicePushState::default();
        // shell.execute classifies High.
        let decision = decide(
            &approval("req-9", Some("t1"), "shell.execute", "rm -rf /secret/path"),
            &mut state,
            0,
        )
        .unwrap();
        assert_eq!(decision.kind, PushKind::Alert);
        assert_eq!(decision.category, Some(CATEGORY_APPROVAL_HIGH));
        assert_eq!(decision.payload["aps"]["category"], CATEGORY_APPROVAL_HIGH);
        assert_eq!(decision.collapse_id.as_deref(), Some("approval-req-9"));
        assert_eq!(
            decision.payload["aps"]["interruption-level"],
            "time-sensitive"
        );
        assert_eq!(decision.payload["tc"]["request_id"], "req-9");
        assert_eq!(decision.payload["tc"]["thread_id"], "t1");
        // Never the tool name or parameters.
        assert_content_free(&decision.payload, &["shell.execute", "rm -rf /secret/path"]);
    }

    #[test]
    fn low_risk_approval_maps_to_low_category_alert() {
        let mut state = DevicePushState::default();
        // read_file classifies Low.
        let decision = decide(
            &approval(
                "req-10",
                Some("t1"),
                "read_file",
                "{\"path\":\"/etc/hosts\"}",
            ),
            &mut state,
            0,
        )
        .unwrap();
        assert_eq!(decision.kind, PushKind::Alert);
        assert_eq!(decision.category, Some(CATEGORY_APPROVAL_LOW));
        assert_eq!(decision.payload["aps"]["category"], CATEGORY_APPROVAL_LOW);
        assert_eq!(decision.collapse_id.as_deref(), Some("approval-req-10"));
    }

    #[test]
    fn approval_category_follows_event_risk_tier() {
        let mut state = DevicePushState::default();
        let high = decide(
            &approval_with_risk("r-hi", ApprovalRisk::High),
            &mut state,
            0,
        )
        .unwrap();
        assert_eq!(high.category, Some(CATEGORY_APPROVAL_HIGH));
        let low = decide(
            &approval_with_risk("r-lo", ApprovalRisk::Low),
            &mut state,
            0,
        )
        .unwrap();
        assert_eq!(low.category, Some(CATEGORY_APPROVAL_LOW));
    }

    #[test]
    fn job_result_maps_to_job_alert() {
        let mut state = DevicePushState::default();
        let decision = decide(
            &job_result("job-7", "build failed: leaked detail"),
            &mut state,
            0,
        )
        .unwrap();
        assert_eq!(decision.kind, PushKind::Alert);
        assert_eq!(decision.category, Some(CATEGORY_JOB));
        assert_eq!(decision.collapse_id.as_deref(), Some("job-job-7"));
        assert_eq!(decision.payload["tc"]["job_id"], "job-7");
        assert_content_free(&decision.payload, &["build failed: leaked detail"]);
    }

    #[test]
    fn run_progress_without_tracked_run_produces_no_push() {
        let mut state = DevicePushState::default();
        assert!(decide(&tool_started("t1", "shell.execute"), &mut state, 0).is_none());
        assert!(decide(&status("t1", "thinking hard"), &mut state, 0).is_none());
        assert!(decide(&context_pressure("t1", "warning", 88.0), &mut state, 0).is_none());
    }

    #[test]
    fn tracked_context_pressure_produces_content_free_live_activity_update() {
        let mut state = DevicePushState::default();
        state.track_run("t1", "act-1");

        let decision = decide(&context_pressure("t1", "critical", 97.0), &mut state, 0)
            .expect("tracked pressure update");

        assert_eq!(decision.kind, PushKind::LiveActivityUpdate);
        assert_eq!(decision.activity_id.as_deref(), Some("act-1"));
        assert_content_free(&decision.payload, &["critical", "97"]);
    }

    #[test]
    fn tracked_run_progress_produces_throttled_live_activity_updates() {
        let mut state = DevicePushState::default();
        state.track_run("t1", "act-1");

        // First update emits at t=0, revision 1.
        let first = decide(&tool_started("t1", "shell.execute"), &mut state, 0).unwrap();
        assert_eq!(first.kind, PushKind::LiveActivityUpdate);
        assert_eq!(first.collapse_id.as_deref(), Some("run-act-1"));
        assert_eq!(first.payload["aps"]["content-state"]["revision"], 1);
        assert_eq!(
            first.payload["aps"]["content-state"]["phase"],
            "runningTool"
        );
        // Tool name never rides in the content-state.
        assert_content_free(&first.payload, &["shell.execute"]);

        // A second update inside the throttle window is suppressed.
        assert!(decide(&status("t1", "still going"), &mut state, 5).is_none());
        assert!(
            decide(
                &status("t1", "still going"),
                &mut state,
                LIVE_ACTIVITY_MIN_INTERVAL_SECS - 1
            )
            .is_none()
        );

        // Past the window, a new update emits with a monotonically higher revision.
        let second = decide(
            &status("t1", "thinking"),
            &mut state,
            LIVE_ACTIVITY_MIN_INTERVAL_SECS,
        )
        .unwrap();
        assert_eq!(second.payload["aps"]["content-state"]["revision"], 2);
        assert_eq!(second.payload["aps"]["content-state"]["phase"], "thinking");
    }

    #[test]
    fn response_closing_tracked_run_ends_live_activity() {
        let mut state = DevicePushState::default();
        state.track_run("t1", "act-1");
        let _ = decide(&tool_started("t1", "x"), &mut state, 0).unwrap(); // revision 1

        let end = decide(&response("t1", "final answer body"), &mut state, 30).unwrap();
        assert_eq!(end.kind, PushKind::LiveActivityEnd);
        assert_eq!(end.collapse_id.as_deref(), Some("run-act-1"));
        assert_eq!(end.payload["aps"]["event"], "end");
        assert_eq!(end.payload["aps"]["content-state"]["phase"], "done");
        // End bumps revision past the last update (2 > 1) and is never throttled.
        assert_eq!(end.payload["aps"]["content-state"]["revision"], 2);
        assert_content_free(&end.payload, &["final answer body"]);
        // The run is no longer tracked; a later response falls back to an alert.
        assert!(!state.is_tracking("t1"));
        let after = decide(&response("t1", "another"), &mut state, 40).unwrap();
        assert_eq!(after.kind, PushKind::Alert);
    }

    #[test]
    fn background_wake_budget_is_bounded_per_hour() {
        let mut state = DevicePushState::default();
        let tc = serde_json::json!({ "thread_id": "t1" });

        // Budget starts full.
        assert_eq!(state.wake_budget_remaining(0), BACKGROUND_WAKE_BUDGET);
        for i in 0..BACKGROUND_WAKE_BUDGET {
            let wake = decide_background_wake(&mut state, u64::from(i), tc.clone());
            assert!(wake.is_some(), "wake {i} should be granted");
            let wake = wake.unwrap();
            assert_eq!(wake.kind, PushKind::Background);
            assert_eq!(wake.payload["aps"]["content-available"], 1);
            assert_content_free(&wake.payload, &["secret"]);
        }
        // Budget exhausted.
        assert!(decide_background_wake(&mut state, 10, tc.clone()).is_none());
        assert_eq!(state.wake_budget_remaining(10), 0);

        // Once the whole window has elapsed past the newest event, the budget
        // fully refills (the last wake was at t=2, so any `now` past
        // 2 + window has pruned all three).
        let later = 2 + BACKGROUND_WAKE_WINDOW_SECS;
        assert_eq!(state.wake_budget_remaining(later), BACKGROUND_WAKE_BUDGET);
        assert!(decide_background_wake(&mut state, later, tc).is_some());
    }

    #[test]
    fn can_produce_push_covers_every_mapping_event() {
        // Events that map to a push must be admitted by the pre-filter.
        let mut state = DevicePushState::default();
        state.track_run("t1", "act-1");
        for event in [
            response("t1", "body"),
            approval("r1", Some("t1"), "tool", "p"),
            job_result("j1", "msg"),
            SseEvent::JobSessionResult {
                job_id: "j1".into(),
                session_id: Some("s1".into()),
                status: "completed".into(),
                success: Some(true),
                message: None,
            },
            tool_started("t1", "shell.execute"),
            status("t1", "thinking"),
            context_pressure("t1", "warning", 88.0),
        ] {
            assert!(
                can_produce_push(&event),
                "pre-filter dropped a pushable event: {event:?}"
            );
        }

        // Streaming/heartbeat churn must be dropped before any device lookup.
        for event in [
            SseEvent::Heartbeat,
            SseEvent::Thinking {
                message: "chain of thought".into(),
                thread_id: Some("t1".into()),
            },
        ] {
            assert!(!can_produce_push(&event), "pre-filter kept: {event:?}");
        }
    }

    #[test]
    fn live_activity_start_is_content_free_and_seeds_activity() {
        let start = live_activity_start("t1", "act-1", 42);
        assert_eq!(start.kind, PushKind::LiveActivityStart);
        assert_eq!(start.activity_id.as_deref(), Some("act-1"));
        assert_eq!(start.collapse_id.as_deref(), Some("run-act-1"));
        assert_eq!(start.payload["aps"]["event"], "start");
        // Initial content-state is the content-free {phase, revision} shape.
        assert_eq!(start.payload["aps"]["content-state"]["phase"], "thinking");
        assert_eq!(start.payload["aps"]["content-state"]["revision"], 0);
        // Attributes carry only the id; never prompt/tool text.
        assert_eq!(start.payload["aps"]["attributes"]["thread_id"], "t1");
        assert_content_free(&start.payload, &["secret", "shell.execute"]);
    }

    #[test]
    fn ensure_tracked_starts_once_and_preserves_revision_on_reassociation() {
        let mut state = DevicePushState::default();
        // First call starts tracking.
        assert!(state.ensure_tracked("t1", "start-t1"));
        // Bump the revision via an update.
        let _ = decide(&tool_started("t1", "x"), &mut state, 0).unwrap(); // revision 1
        // Re-associating to the real activity id does not restart tracking and
        // preserves the revision.
        assert!(!state.ensure_tracked("t1", "act-real"));
        assert_eq!(state.tracked_activity_id("t1"), Some("act-real"));
        let next = decide(
            &status("t1", "thinking"),
            &mut state,
            LIVE_ACTIVITY_MIN_INTERVAL_SECS,
        )
        .unwrap();
        // Revision continues from 1 → 2 under the new activity id.
        assert_eq!(next.payload["aps"]["content-state"]["revision"], 2);
        assert_eq!(next.activity_id.as_deref(), Some("act-real"));
    }

    #[test]
    fn untrack_run_clears_start_marker_so_a_new_run_can_restart() {
        let mut state = DevicePushState::default();
        assert!(state.mark_start_emitted("t1"), "first mark is fresh");
        assert!(!state.mark_start_emitted("t1"), "second is a no-op");
        state.untrack_run("t1");
        assert!(
            state.mark_start_emitted("t1"),
            "after a run ends, a new run may start a fresh activity"
        );
    }

    #[test]
    fn unmapped_events_produce_no_push() {
        let mut state = DevicePushState::default();
        assert!(decide(&SseEvent::Heartbeat, &mut state, 0).is_none());
        assert!(
            decide(
                &SseEvent::Thinking {
                    message: "secret chain of thought".into(),
                    thread_id: Some("t1".into())
                },
                &mut state,
                0
            )
            .is_none()
        );
    }
}
