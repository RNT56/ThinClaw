import Foundation
import Testing
import ThinClawSnapshotKit

@testable import ThinClawCore

// MARK: - Test doubles

/// Records every write and counts them, so tests assert on coalescing.
/// A lock-guarded class (not an actor) because `SnapshotSink.write` is a
/// synchronous `nonisolated` requirement an actor cannot satisfy.
private final class RecordingSink: SnapshotSink, @unchecked Sendable {
    private let lock = NSLock()
    private var writes: [ProjectedSnapshots] = []
    func write(_ snapshots: ProjectedSnapshots) throws { lock.withLock { writes.append(snapshots) } }
    var count: Int { lock.withLock { writes.count } }
    var last: ProjectedSnapshots? { lock.withLock { writes.last } }
}

/// A clock whose `sleep` completes only when the test releases a gate, so the
/// debounce fires deterministically rather than on a wall-clock timer. `now()`
/// returns a fixed instant unless advanced.
private final class GateClock: SnapshotClock, @unchecked Sendable {
    private let lock = NSLock()
    private var fixedNow: Date
    private var gates: [CheckedContinuation<Void, any Error>] = []

    init(now: Date = Date(timeIntervalSince1970: 1_000_000)) { self.fixedNow = now }

    func now() -> Date {
        lock.withLock { fixedNow }
    }

    func setNow(_ date: Date) { lock.withLock { fixedNow = date } }

    func sleep(for duration: Duration) async throws {
        try await withCheckedThrowingContinuation { cont in
            lock.withLock { gates.append(cont) }
        }
    }

    /// Release all currently-waiting sleepers (fires the pending debounce).
    /// Returns how many were released.
    @discardableResult
    func fire() -> Int {
        let waiting = lock.withLock { () -> [CheckedContinuation<Void, any Error>] in
            let w = gates
            gates = []
            return w
        }
        for cont in waiting { cont.resume() }
        return waiting.count
    }

    var pendingSleepers: Int { lock.withLock { gates.count } }
}

/// Keep firing the gate until at least `minWrites` writes have landed (or a
/// deadline passes). The publisher reschedules its debounce on every edit, so a
/// burst can leave a sleeper registered *after* an earlier `fire()` drained the
/// (now-cancelled) previous one; draining repeatedly closes that race
/// deterministically without depending on wall-clock timing.
private func drain(
    _ clock: GateClock, until sink: RecordingSink, minWrites: Int = 1,
    timeout: Duration = .seconds(2)
) async {
    let deadline = ContinuousClock.now.advanced(by: timeout)
    while ContinuousClock.now < deadline {
        clock.fire()
        if sink.count >= minWrites { return }
        try? await Task.sleep(for: .milliseconds(5))
    }
}

private func makeInputs(unread: Int) -> SnapshotInputs {
    SnapshotInputs(phase: .idle, unreadCount: unread)
}

/// Spin until `condition` holds or a generous deadline passes, so tests do not
/// race the publisher's detached debounce task.
private func eventually(
    _ condition: @Sendable () async -> Bool,
    timeout: Duration = .seconds(2)
) async {
    let deadline = ContinuousClock.now.advanced(by: timeout)
    while ContinuousClock.now < deadline {
        if await condition() { return }
        try? await Task.sleep(for: .milliseconds(5))
    }
}

// MARK: - Tests

@Suite("SnapshotPublisher folding + debounce")
struct SnapshotPublisherTests {
    @Test("Folds a tool_started event into runningTool with the tool name")
    func foldsToolStarted() async {
        let sink = RecordingSink()
        let clock = GateClock()
        let publisher = SnapshotPublisher(
            sink: sink, privacy: .default, clock: clock, debounceInterval: .milliseconds(1))

        await publisher.ingest(
            event: .toolStarted(name: "shell_command", threadID: ThreadID("t1")),
            threadTitle: "Build")
        await drain(clock, until: sink)

        let last = try! #require(sink.last)
        #expect(last.status.phase == .runningTool)
        #expect(last.status.activeToolName == "shell_command")
        #expect(last.status.activeThreadID == "t1")
        #expect(last.status.activeThreadTitle == "Build")
    }

    @Test("Coalesces a burst of events into a single write")
    func coalescesBurst() async {
        let sink = RecordingSink()
        let clock = GateClock()
        let publisher = SnapshotPublisher(
            sink: sink, privacy: .default, clock: clock, debounceInterval: .milliseconds(1))

        // Five rapid events; each cancels the previous debounce timer.
        for i in 0..<5 {
            await publisher.ingest(
                event: .thinking(message: "step \(i)", threadID: ThreadID("t1")))
        }
        // Exactly one sleeper should survive (the latest schedule).
        await drain(clock, until: sink)
        // Give any stragglers a beat; the coalesced count must stay at 1.
        try? await Task.sleep(for: .milliseconds(20))
        #expect(sink.count == 1)
        #expect(sink.last?.status.phase == .thinking)
    }

    @Test("A response increments unread and returns to idle")
    func responseCountsUnread() async {
        let sink = RecordingSink()
        let clock = GateClock()
        let publisher = SnapshotPublisher(
            sink: sink, privacy: .default, clock: clock, debounceInterval: .milliseconds(1))

        await publisher.ingest(event: .response(content: "hi", threadID: ThreadID("t1")))
        await publisher.ingest(event: .response(content: "again", threadID: ThreadID("t1")))
        await drain(clock, until: sink)

        #expect(sink.last?.status.phase == .idle)
        #expect(sink.last?.status.unreadCount == 2)
    }

    @Test("Heartbeat and unknown events never trigger a write")
    func ignoresNoiseEvents() async {
        let sink = RecordingSink()
        let clock = GateClock()
        let publisher = SnapshotPublisher(
            sink: sink, privacy: .default, clock: clock, debounceInterval: .milliseconds(1))

        await publisher.ingest(event: .heartbeat)
        await publisher.ingest(event: .unknown(type: "plan_update"))
        // No debounce should have been scheduled at all.
        try? await Task.sleep(for: .milliseconds(20))
        #expect(clock.pendingSleepers == 0)
        #expect(sink.count == 0)
    }

    @Test("setApprovals promotes an idle phase to waitingForApproval")
    func approvalsPromotePhase() async {
        let sink = RecordingSink()
        let clock = GateClock()
        let publisher = SnapshotPublisher(
            sink: sink, privacy: .default, clock: clock, debounceInterval: .milliseconds(1))

        await publisher.setApprovals([
            ApprovalRequest(
                requestID: "r1", toolName: "run", description: "do it",
                parameters: "{}", risk: .low)
        ])
        await drain(clock, until: sink)

        #expect(sink.last?.status.phase == .waitingForApproval)
        #expect(sink.last?.approvals.approvals.count == 1)
    }

    @Test("publishNow writes immediately, bypassing the debounce")
    func publishNowBypassesDebounce() async throws {
        let sink = RecordingSink()
        let clock = GateClock()
        let publisher = SnapshotPublisher(
            sink: sink, privacy: .default, clock: clock, debounceInterval: .seconds(30))

        try await publisher.publishNow(makeInputs(unread: 7))

        // No sleep involved; the write is synchronous within the actor call.
        #expect(sink.count == 1)
        #expect(sink.last?.status.unreadCount == 7)
        #expect(clock.pendingSleepers == 0)
    }

    @Test("Identical inputs are not re-written (no-op suppression)")
    func suppressesNoOpWrites() async throws {
        let sink = RecordingSink()
        let clock = GateClock()
        let publisher = SnapshotPublisher(
            sink: sink, privacy: .default, clock: clock, debounceInterval: .milliseconds(1))

        try await publisher.publishNow(makeInputs(unread: 3))
        #expect(sink.count == 1)
        // Same inputs again → suppressed.
        try await publisher.publishNow(makeInputs(unread: 3))
        #expect(sink.count == 1)
        // Changed inputs → written.
        try await publisher.publishNow(makeInputs(unread: 4))
        #expect(sink.count == 2)
    }

    @Test("Publisher → SnapshotStore integration round-trips the three files")
    func storeIntegration() async throws {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("thinclaw-pub-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let store = SnapshotStore(baseURL: dir)
        let sink = SnapshotStoreSink(store: store)
        let clock = GateClock(now: Date(timeIntervalSince1970: 1_750_000_000))
        clock.setNow(Date(timeIntervalSince1970: 1_750_000_000))
        let publisher = SnapshotPublisher(
            sink: sink, privacy: .default, clock: clock, debounceInterval: .milliseconds(1))

        let inputs = SnapshotInputs(
            phase: .runningTool,
            activeToolName: "grep",
            activeThreadID: ThreadID("t9"),
            activeThreadTitle: "Search",
            unreadCount: 1,
            pendingApprovals: [
                ApprovalRequest(
                    requestID: "r1", toolName: "write", description: "write it",
                    parameters: "{}", risk: .high, threadID: ThreadID("t9"))
            ],
            jobs: [
                .init(
                    id: "j1", title: "Job one", phase: .running,
                    startedAt: Date(timeIntervalSince1970: 1_749_000_000))
            ])
        try await publisher.publishNow(inputs)

        let status = try #require(try store.load(AgentStatusSnapshot.self))
        #expect(status.phase == .runningTool)
        #expect(status.activeToolName == "grep")
        #expect(status.activeThreadTitle == "Search")

        let approvals = try #require(try store.load(PendingApprovalsSnapshot.self))
        #expect(approvals.approvals.first?.risk == .high)
        #expect(approvals.approvals.first?.id == "r1")

        let jobs = try #require(try store.load(JobsSnapshot.self))
        #expect(jobs.jobs.first?.title == "Job one")
        #expect(jobs.jobs.first?.phase == .running)
    }
}
