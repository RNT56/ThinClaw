import Foundation
import ThinClawCore

/// Owns the agent-run Live Activity for the active thread: it observes a
/// ``RunEventSource``'s events, drives the pure ``RunTracker`` to decide
/// start/update/end + a monotonic revision, and performs those against an
/// ``ActivityController`` (ActivityKit) while registering per-activity push
/// tokens and the push-to-start token through a ``LiveActivityRegistrar``.
///
/// One activity per thread at most (the reducer's invariant). Local SSE updates
/// drive the activity while foregrounded — lower latency than a push — and each
/// carries a strictly increasing revision so a late gateway push can be dropped
/// by the widget (docs/MOBILE_SECURITY.md D-N2).
///
/// Lifecycle: ``observe(thread:)`` starts watching a thread's events; a new call
/// switches the active thread (ending nothing — a backgrounded run keeps its
/// activity, but only the active thread's events are tracked here). ``stop()``
/// cancels observation and ends every activity (used on unpair/teardown).
@MainActor
public final class LiveActivityManager {
    private let eventSource: any RunEventSource
    private let controller: any ActivityController
    private let registrar: any LiveActivityRegistrar

    private var tracker = RunTracker()

    /// The thread currently being observed, if any.
    private(set) var activeThread: ThreadID?
    /// Title to stamp on a fresh activity for the active thread.
    private var activeThreadTitle: String = ""

    /// Reverse map so `end` can `DELETE` the right registration path segment.
    private var activityIDsByThread: [ThreadID: String] = [:]

    /// The event-observation task for the active thread.
    private var observationTask: Task<Void, Never>?
    /// The push-to-start token observation task (device-wide, started once).
    private var startTokenTask: Task<Void, Never>?

    public init(
        eventSource: any RunEventSource,
        controller: any ActivityController,
        registrar: any LiveActivityRegistrar
    ) {
        self.eventSource = eventSource
        self.controller = controller
        self.registrar = registrar
    }

    // MARK: - Push-to-start

    /// Begin forwarding ActivityKit's push-to-start token updates to the gateway
    /// (`PUT /api/devices/me/live-activity-start-token`) so a killed app can be
    /// spawned by the gateway's push-to-start. Idempotent; call once while
    /// paired+active. The controller owns the real
    /// `Activity.pushToStartTokenUpdates` sequence.
    public func startPushToStartRegistration(
        tokens: @escaping @Sendable () -> AsyncStream<String>
    ) {
        guard startTokenTask == nil else { return }
        startTokenTask = Task { [registrar] in
            for await token in tokens() {
                if Task.isCancelled { break }
                await registrar.registerStartToken(pushToken: token)
            }
        }
    }

    // MARK: - Observation

    /// Observe `thread`'s live events and drive its Live Activity. Switching to
    /// a new thread cancels the previous observation (the previous thread's
    /// activity, if any, is left running until it completes) and starts watching
    /// the new one. No-op if already observing `thread`.
    public func observe(thread: ThreadID, title: String) {
        if activeThread == thread {
            activeThreadTitle = title
            return
        }
        observationTask?.cancel()
        activeThread = thread
        activeThreadTitle = title

        observationTask = Task { [weak self, eventSource] in
            let events = await eventSource.events(in: thread)
            for await event in events {
                if Task.isCancelled { break }
                await self?.handle(event: event)
            }
        }
    }

    /// Stop observing and end every activity. Cancels the observation and
    /// push-to-start tasks. Used on unpair or when the app is torn down.
    public func stop() async {
        observationTask?.cancel()
        observationTask = nil
        startTokenTask?.cancel()
        startTokenTask = nil
        activeThread = nil
        activeThreadTitle = ""
        tracker = RunTracker()
        activityIDsByThread.removeAll()
        await controller.endAll()
    }

    /// Test seam: set the active thread/title without launching an observation
    /// task, so unit tests can drive ``handle(event:)`` directly against a
    /// scripted event set.
    func beginObservingForTests(thread: ThreadID, title: String) {
        activeThread = thread
        activeThreadTitle = title
    }

    // MARK: - Event handling

    /// Feed one event through the classifier + reducer and perform the action.
    /// `internal` so tests can drive the manager without a live SSE stream.
    func handle(event: AgentEvent) async {
        guard let thread = activeThread else { return }
        guard
            let input = RunInputClassifier.input(
                from: event, activeThread: thread, threadTitle: activeThreadTitle)
        else { return }
        guard let action = tracker.reduce(input) else { return }
        await perform(action)
    }

    /// Record that a late push revision landed for the active thread, so the
    /// next local update outranks it (the widget keeps the highest revision it
    /// has seen). Called by the app layer when a Live Activity update push is
    /// observed while foregrounded.
    public func notePushRevision(_ revision: UInt64) {
        guard let thread = activeThread else { return }
        _ = tracker.reduce(.pushRevisionObserved(threadID: thread, revision: revision))
    }

    private func perform(_ action: RunAction) async {
        switch action {
        case let .start(threadID, threadTitle, state):
            guard controller.areActivitiesEnabled else {
                // Live Activities disabled: drop the tracked run so a later
                // signal re-attempts a start rather than emitting orphan
                // updates against an activity that was never created.
                _ = tracker.reduce(.completed(threadID: threadID))
                return
            }
            if let activityID = await controller.requestActivity(
                thread: threadID, title: threadTitle, state: state)
            {
                activityIDsByThread[threadID] = activityID
            }

        case let .update(threadID, state):
            await controller.updateActivity(thread: threadID, state: state)

        case let .end(threadID, state):
            await controller.endActivity(thread: threadID, state: state)
            if let activityID = activityIDsByThread.removeValue(forKey: threadID) {
                await registrar.removeActivityToken(activityID: activityID)
            }
        }
    }

    // MARK: - Push-token forwarding (from the controller)

    /// The controller calls this when ActivityKit hands it a new per-activity
    /// push token, so the manager can register it against the right activity id
    /// + thread (`PUT /api/devices/me/live-activity/{activity_id}`).
    public func registerActivityPushToken(
        activityID: String,
        pushToken: String,
        thread: ThreadID
    ) async {
        await registrar.registerActivityToken(
            activityID: activityID, pushToken: pushToken, threadID: thread)
    }
}
