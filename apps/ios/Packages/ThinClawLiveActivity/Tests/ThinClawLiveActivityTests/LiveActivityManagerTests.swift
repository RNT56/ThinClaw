import Foundation
import Testing
import ThinClawCore

@testable import ThinClawLiveActivity

/// Drives ``LiveActivityManager`` through fakes for the ActivityKit controller,
/// the gateway registrar, and the event source, asserting the start-once /
/// end-on-completion / disabled-guard / registration behaviour without touching
/// ActivityKit (so this runs under `swift test` on a Mac host).
@MainActor
@Suite("LiveActivityManager")
struct LiveActivityManagerTests {
    private let thread = ThreadID("web-1")

    private func makeManager(
        activitiesEnabled: Bool = true
    ) -> (LiveActivityManager, FakeActivityController, FakeRegistrar) {
        let controller = FakeActivityController(enabled: activitiesEnabled)
        let registrar = FakeRegistrar()
        let manager = LiveActivityManager(
            eventSource: EmptyEventSource(),
            controller: controller,
            registrar: registrar)
        manager.beginObservingForTests(thread: thread, title: "Chat")
        return (manager, controller, registrar)
    }

    @Test("a run start requests exactly one activity")
    func startRequestsActivity() async {
        let (manager, controller, _) = makeManager()
        await manager.handle(event: .thinking(message: "…", threadID: thread))
        #expect(controller.requested.count == 1)
        #expect(controller.requested[0].thread == thread)
        #expect(controller.requested[0].state.phase == .thinking)
        // A second start signal updates, never re-requests.
        await manager.handle(event: .toolStarted(name: "grep", threadID: thread))
        #expect(controller.requested.count == 1)
        #expect(controller.updated.count == 1)
    }

    @Test("completion ends the activity and deletes the registration")
    func completionEndsAndDeletes() async {
        let (manager, controller, registrar) = makeManager()
        // Give the activity a push token so there is a registration to delete.
        await manager.handle(event: .toolStarted(name: "t", threadID: thread))
        let activityID = controller.requested[0].activityID
        await manager.registerActivityPushToken(
            activityID: activityID, pushToken: "deadbeef", thread: thread)
        #expect(registrar.registered.count == 1)
        #expect(registrar.registered[0].activityID == activityID)
        #expect(registrar.registered[0].threadID == thread)

        await manager.handle(event: .response(content: "done", threadID: thread))
        #expect(controller.ended.count == 1)
        #expect(controller.ended[0].state.phase == .done)
        #expect(registrar.removed == [activityID])
    }

    @Test("with activities disabled, no activity is requested and the run is dropped")
    func disabledGuard() async {
        let (manager, controller, _) = makeManager(activitiesEnabled: false)
        await manager.handle(event: .thinking(message: "…", threadID: thread))
        #expect(controller.requested.isEmpty)
        // The dropped run means the next start signal re-attempts a request
        // (rather than silently updating a non-existent activity).
        await manager.handle(event: .toolStarted(name: "t", threadID: thread))
        #expect(controller.requested.isEmpty)
    }

    @Test("stop ends all activities and clears state")
    func stopEndsAll() async {
        let (manager, controller, _) = makeManager()
        await manager.handle(event: .thinking(message: "…", threadID: thread))
        await manager.stop()
        #expect(controller.endAllCount == 1)
    }

    @Test("a late push revision keeps the next local update ahead")
    func pushRevisionThroughManager() async {
        let (manager, controller, _) = makeManager()
        await manager.handle(event: .thinking(message: "…", threadID: thread))  // rev 1
        manager.notePushRevision(9)
        await manager.handle(event: .toolStarted(name: "t", threadID: thread))
        #expect(controller.updated.last?.state.revision == 10)
    }

    @Test("push-to-start tokens are forwarded to the registrar")
    func pushToStartForwarded() async {
        let (manager, _, registrar) = makeManager()
        let (stream, continuation) = AsyncStream<String>.makeStream()
        manager.startPushToStartRegistration(tokens: { stream })
        continuation.yield("startbeef")
        continuation.finish()
        // Let the forwarding task drain.
        try? await Task.sleep(for: .milliseconds(50))
        #expect(registrar.startTokens == ["startbeef"])
    }
}

// MARK: - Fakes

/// A ``RunEventSource`` that never emits — the manager tests drive `handle`
/// directly, so observation is not exercised here.
private struct EmptyEventSource: RunEventSource {
    func events(in thread: ThreadID) async -> AsyncStream<AgentEvent> {
        AsyncStream { $0.finish() }
    }
}

@MainActor
private final class FakeActivityController: ActivityController {
    struct Request: Sendable {
        let thread: ThreadID
        let state: RunState
        let activityID: String
    }
    struct Update: Sendable {
        let thread: ThreadID
        let state: RunState
    }

    let enabled: Bool
    private(set) var requested: [Request] = []
    private(set) var updated: [Update] = []
    private(set) var ended: [Update] = []
    private(set) var endAllCount = 0
    private var nextID = 0

    init(enabled: Bool) { self.enabled = enabled }

    var areActivitiesEnabled: Bool { enabled }

    func requestActivity(thread: ThreadID, title: String, state: RunState) async -> String? {
        nextID += 1
        let id = "activity-\(nextID)"
        requested.append(Request(thread: thread, state: state, activityID: id))
        return id
    }

    func updateActivity(thread: ThreadID, state: RunState) async {
        updated.append(Update(thread: thread, state: state))
    }

    func endActivity(thread: ThreadID, state: RunState) async {
        ended.append(Update(thread: thread, state: state))
    }

    func endAll() async { endAllCount += 1 }
}

private final class FakeRegistrar: LiveActivityRegistrar, @unchecked Sendable {
    struct Registration: Sendable {
        let activityID: String
        let pushToken: String
        let threadID: ThreadID
    }
    private(set) var registered: [Registration] = []
    private(set) var removed: [String] = []
    private(set) var startTokens: [String] = []

    func registerActivityToken(
        activityID: String, pushToken: String, threadID: ThreadID
    ) async {
        registered.append(
            Registration(activityID: activityID, pushToken: pushToken, threadID: threadID))
    }

    func removeActivityToken(activityID: String) async {
        removed.append(activityID)
    }

    func registerStartToken(pushToken: String) async {
        startTokens.append(pushToken)
    }
}
