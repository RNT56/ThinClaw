import Foundation
import ThinClawCore

/// The ActivityKit surface the ``LiveActivityManager`` needs, expressed as a
/// protocol so tests can fake it (ActivityKit is iOS-only and cannot run under
/// `swift test` on a Mac host). The production conformance
/// (`LiveActivityKitController`, iOS-only) wraps `ActivityKit.Activity`.
///
/// The controller owns the actual `Activity` handles keyed by thread; the
/// manager drives it with the content-free ``RunState`` the reducer produces
/// and never touches `ActivityKit` types directly.
@MainActor
public protocol ActivityController: AnyObject {
    /// Whether the user has Live Activities enabled for this app
    /// (`ActivityAuthorizationInfo().areActivitiesEnabled`). The manager guards
    /// every `request` on this.
    var areActivitiesEnabled: Bool { get }

    /// Request a new Activity for `thread` with `title` and the initial
    /// `state`, asking ActivityKit for a `.token` push type. Returns the
    /// activity id ActivityKit assigns (used as the registration path segment),
    /// or `nil` if the request failed. Implementations begin observing
    /// `pushTokenUpdates` for the new activity and forward them via
    /// ``LiveActivityManager`` -> the registrar.
    func requestActivity(
        thread: ThreadID,
        title: String,
        state: RunState
    ) async -> String?

    /// Locally update the Activity for `thread` to `state`. Lower latency than
    /// a push; carries the monotonic revision so the widget drops a late push.
    func updateActivity(thread: ThreadID, state: RunState) async

    /// End the Activity for `thread` with `state` and a dismissal policy, then
    /// forget its handle. Idempotent for an unknown thread.
    func endActivity(thread: ThreadID, state: RunState) async

    /// End and forget *every* tracked activity (used on unpair/teardown).
    func endAll() async
}

/// The gateway-side Live Activity token registration the manager needs. Wraps
/// the four generated client operations behind a small protocol so the manager
/// does not depend on the OpenAPI client directly and tests can assert the
/// exact registrations.
public protocol LiveActivityRegistrar: Sendable {
    /// `PUT /api/devices/me/live-activity/{activity_id}` — register (or replace)
    /// the per-activity update-push token. `pushToken` is the hex APNs token
    /// from ActivityKit's `pushTokenUpdates`. For an agent run, pass
    /// `threadID`; the gateway routes run events to this token (D-N2).
    func registerActivityToken(
        activityID: String,
        pushToken: String,
        threadID: ThreadID
    ) async

    /// `DELETE /api/devices/me/live-activity/{activity_id}` — drop the
    /// per-activity token when the activity ends.
    func removeActivityToken(activityID: String) async

    /// `PUT /api/devices/me/live-activity-start-token` — register the device's
    /// push-to-start token so the gateway can spawn a killed app's activity.
    func registerStartToken(pushToken: String) async
}
