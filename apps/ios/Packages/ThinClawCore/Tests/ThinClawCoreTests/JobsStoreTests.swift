import Foundation
import Testing

@testable import ThinClawCore

// MARK: - Test doubles

/// Scripts list/summary/detail responses and serves job-event snapshots from a
/// queue, so the store's mapping and the id-cursor fold are exercised without a
/// live gateway. Every method can be made to throw to drive error/backoff paths.
private final class MockJobsGateway: JobsGateway, @unchecked Sendable {
    private let lock = NSLock()

    private var _jobs: [Job]
    private var _summary: JobsSummary
    private var _detail: JobDetail?
    private var _detailSequence: [JobDetail] = []
    private var _eventSnapshots: [[JobEvent]] = []
    private var _throwList = false
    private var _throwSummary = false
    private var _throwEventsOnce = false

    private(set) var listCalls = 0
    private(set) var eventCalls = 0
    private(set) var detailCalls = 0

    init(
        jobs: [Job] = [],
        summary: JobsSummary = JobsSummary(),
        detail: JobDetail? = nil
    ) {
        self._jobs = jobs
        self._summary = summary
        self._detail = detail
    }

    // Scripting knobs.
    func setJobs(_ jobs: [Job]) { lock.withLock { _jobs = jobs } }
    func setThrowList(_ value: Bool) { lock.withLock { _throwList = value } }
    func setThrowSummary(_ value: Bool) { lock.withLock { _throwSummary = value } }
    func setThrowEventsOnce(_ value: Bool) { lock.withLock { _throwEventsOnce = value } }
    func enqueueEventSnapshot(_ events: [JobEvent]) {
        lock.withLock { _eventSnapshots.append(events) }
    }
    func resetEventSnapshots(_ events: [JobEvent]) {
        lock.withLock { _eventSnapshots = [events] }
    }
    func setDetailSequence(_ details: [JobDetail]) {
        lock.withLock { _detailSequence = details }
    }

    struct Failure: Error {}

    func listJobs() async throws -> [Job] {
        try lock.withLock {
            listCalls += 1
            if _throwList { throw Failure() }
            return _jobs
        }
    }

    func jobsSummary() async throws -> JobsSummary {
        try lock.withLock {
            if _throwSummary { throw Failure() }
            return _summary
        }
    }

    func jobDetail(id: String) async throws -> JobDetail {
        try lock.withLock {
            detailCalls += 1
            if !_detailSequence.isEmpty {
                let next = _detailSequence.removeFirst()
                _detail = next
                return next
            }
            guard let detail = _detail else { throw Failure() }
            return detail
        }
    }

    func jobEvents(id: String) async throws -> [JobEvent] {
        try lock.withLock {
            eventCalls += 1
            if _throwEventsOnce {
                _throwEventsOnce = false
                throw Failure()
            }
            // Serve the next scripted snapshot; once drained, repeat the last so
            // the poll loop keeps observing the terminal snapshot.
            if _eventSnapshots.count > 1 {
                return _eventSnapshots.removeFirst()
            }
            return _eventSnapshots.first ?? []
        }
    }
}

/// A clock whose `sleep` blocks until the test releases one "tick", so the poll
/// loop advances exactly one iteration per `tick()` — no wall-clock waits, fully
/// deterministic. Records the requested delays so backoff can be asserted.
private final class ManualJobsClock: JobsClock, @unchecked Sendable {
    private let lock = NSLock()
    private var waiters: [CheckedContinuation<Void, Never>] = []
    private var pendingTicks = 0
    private(set) var delays: [Duration] = []

    func sleep(for duration: Duration) async throws {
        try Task.checkCancellation()
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            lock.lock()
            delays.append(duration)
            if pendingTicks > 0 {
                pendingTicks -= 1
                lock.unlock()
                continuation.resume()
            } else {
                waiters.append(continuation)
                lock.unlock()
            }
        }
        try Task.checkCancellation()
    }

    /// Release one sleeping poll iteration (or bank a tick for the next sleep).
    func tick() {
        lock.lock()
        if waiters.isEmpty {
            pendingTicks += 1
            lock.unlock()
        } else {
            let continuation = waiters.removeFirst()
            lock.unlock()
            continuation.resume()
        }
    }

    var recordedDelays: [Duration] { lock.withLock { delays } }
}

// MARK: - Fixtures

/// The exact JSON shape emitted by `GET /api/jobs/{id}/events`
/// (`crates/thinclaw-gateway/src/web/jobs.rs`, `JobEventsResponse`): a `job_id`
/// plus an `events` array of `{ id, event_type, data, created_at }`. Used to
/// prove the hand-rolled decode + projection matches the backend.
private let jobEventsFixture = """
    {
      "job_id": "00000000-0000-0000-0000-000000000000",
      "events": [
        { "id": 1, "event_type": "message", "data": { "role": "assistant", "content": "Starting up the plan" }, "created_at": "2026-06-02T00:00:00+00:00" },
        { "id": 2, "event_type": "tool_use", "data": { "tool_name": "shell", "input": "ls -la" }, "created_at": "2026-06-02T00:00:01+00:00" },
        { "id": 3, "event_type": "tool_result", "data": { "tool_name": "shell", "success": true, "output": "a\\nb" }, "created_at": "2026-06-02T00:00:02+00:00" },
        { "id": 4, "event_type": "result", "data": { "success": true, "message": "Job completed successfully" }, "created_at": "2026-06-02T00:00:03+00:00" }
      ]
    }
    """

private func decodeEvents(_ json: String) throws -> [JobEvent] {
    let wire = try JSONDecoder().decode(JobEventsWire.self, from: Data(json.utf8))
    return wire.events.map(JobEventProjector.project)
}

private func makeJob(id: String, title: String = "Job", state: String) -> Job {
    Job(
        id: id, title: title, state: state, phase: .from(state: state),
        createdAt: nil, startedAt: nil)
}

private func makeDetail(id: String, state: String, transitions: [JobTransition] = []) -> JobDetail {
    JobDetail(
        id: id, title: "Job", description: "desc", state: state,
        phase: .from(state: state), createdAt: nil, startedAt: nil, completedAt: nil,
        elapsedSeconds: nil, transitions: transitions)
}

// MARK: - Phase mapping

@Suite("JobPhase mapping")
struct JobPhaseTests {
    @Test("gateway state strings collapse into the glance buckets")
    func stateBuckets() {
        #expect(JobPhase.from(state: "pending") == .pending)
        #expect(JobPhase.from(state: "creating") == .pending)
        #expect(JobPhase.from(state: "running") == .running)
        #expect(JobPhase.from(state: "in_progress") == .running)
        #expect(JobPhase.from(state: "completed") == .succeeded)
        #expect(JobPhase.from(state: "failed") == .failed)
        #expect(JobPhase.from(state: "interrupted") == .failed)
        #expect(JobPhase.from(state: "cancelled") == .cancelled)
        #expect(JobPhase.from(state: "stuck") == .stuck)
        #expect(JobPhase.from(state: "wat") == .unknown)
    }

    @Test("terminal phases are exactly the finished buckets")
    func terminality() {
        #expect(JobPhase.succeeded.isTerminal)
        #expect(JobPhase.failed.isTerminal)
        #expect(JobPhase.cancelled.isTerminal)
        #expect(JobPhase.stuck.isTerminal)
        #expect(!JobPhase.pending.isTerminal)
        #expect(!JobPhase.running.isTerminal)
        #expect(!JobPhase.unknown.isTerminal)  // fail-open: never treat unknown as done
    }
}

// MARK: - Event decode + projection (fixture shape)

@Suite("JobEvent projection")
struct JobEventProjectionTests {
    @Test("decodes the backend JobEventsResponse fixture and projects summaries")
    func projectsFixture() throws {
        let events = try decodeEvents(jobEventsFixture)
        #expect(events.count == 4)

        #expect(events[0].kind == .message)
        #expect(events[0].summary == "Starting up the plan")

        #expect(events[1].kind == .toolUse)
        #expect(events[1].summary == "shell")

        #expect(events[2].kind == .toolResult)
        #expect(events[2].summary == "shell — ok")

        #expect(events[3].kind == .result)
        #expect(events[3].summary == "Job completed successfully")

        // Ids are the monotonic cursor and RFC3339 parses.
        #expect(events.map(\.id) == [1, 2, 3, 4])
        #expect(events[0].createdAt != nil)
    }

    @Test("a failed tool_result reads its success flag")
    func toolResultFailure() {
        let wire = JobEventWire(
            id: 9, eventType: "tool_result",
            data: .object(["tool_name": .string("http"), "success": .bool(false)]),
            createdAt: "2026-06-02T00:00:00+00:00")
        #expect(JobEventProjector.project(wire).summary == "http — failed")
    }

    @Test("unknown event types still project without dropping the row")
    func unknownType() {
        let wire = JobEventWire(
            id: 5, eventType: "heartbeat", data: .object([:]),
            createdAt: "2026-06-02T00:00:00+00:00")
        let event = JobEventProjector.project(wire)
        #expect(event.kind == .other)
        #expect(event.summary == "heartbeat")
    }

    @Test("long message bodies are truncated to the char limit")
    func truncation() {
        let long = String(repeating: "x", count: JobEventProjector.summaryCharLimit + 50)
        let wire = JobEventWire(
            id: 6, eventType: "message", data: .object(["content": .string(long)]),
            createdAt: "2026-06-02T00:00:00+00:00")
        let summary = JobEventProjector.project(wire).summary
        #expect(summary.count == JobEventProjector.summaryCharLimit + 1)  // + ellipsis
        #expect(summary.hasSuffix("…"))
    }
}

// MARK: - Poll policy

@Suite("JobsPollPolicy backoff")
struct JobsPollPolicyTests {
    @Test("no failures uses the steady-state interval")
    func steadyState() {
        let policy = JobsPollPolicy.default
        #expect(policy.delay(consecutiveFailures: 0) == .seconds(2))
    }

    @Test("failures back off geometrically, capped at maxInterval")
    func backoff() {
        let policy = JobsPollPolicy(
            interval: .seconds(2), maxInterval: .seconds(30), multiplier: 2)
        #expect(policy.delay(consecutiveFailures: 1) == .seconds(4))
        #expect(policy.delay(consecutiveFailures: 2) == .seconds(8))
        #expect(policy.delay(consecutiveFailures: 3) == .seconds(16))
        // 2 * 2^4 = 32 > 30 → capped.
        #expect(policy.delay(consecutiveFailures: 4) == .seconds(30))
        #expect(policy.delay(consecutiveFailures: 10) == .seconds(30))
    }
}

// MARK: - Store: list

@MainActor
@Suite("JobsStore list")
struct JobsStoreListTests {
    @Test("refresh populates the list and summary from the gateway")
    func refreshLoadsListAndSummary() async {
        let gateway = MockJobsGateway(
            jobs: [makeJob(id: "a", state: "running"), makeJob(id: "b", state: "completed")],
            summary: JobsSummary(total: 2, inProgress: 1, completed: 1))
        let store = JobsStore(gateway: gateway, clock: ManualJobsClock())

        await store.refresh()

        #expect(store.jobs.map(\.id) == ["a", "b"])
        #expect(store.jobs[0].phase == .running)
        #expect(store.summary?.total == 2)
        #expect(store.summary?.active == 1)
        #expect(store.listError == nil)
    }

    @Test("a list failure surfaces an error and keeps the last-known list")
    func listFailureKeepsList() async {
        let gateway = MockJobsGateway(jobs: [makeJob(id: "a", state: "running")])
        let store = JobsStore(gateway: gateway, clock: ManualJobsClock())
        await store.refresh()
        #expect(store.jobs.count == 1)

        gateway.setThrowList(true)
        await store.refresh()

        #expect(store.jobs.count == 1)  // unchanged
        #expect(store.listError != nil)
    }

    @Test("a summary failure does not clobber a good list")
    func summaryFailureKeepsList() async {
        let gateway = MockJobsGateway(jobs: [makeJob(id: "a", state: "running")])
        gateway.setThrowSummary(true)
        let store = JobsStore(gateway: gateway, clock: ManualJobsClock())

        await store.refresh()

        #expect(store.jobs.count == 1)
        #expect(store.listError == nil)
        #expect(store.summary == nil)
    }

    @Test("the store is read-only by construction")
    func readOnly() {
        let store = JobsStore(gateway: MockJobsGateway(), clock: ManualJobsClock())
        #expect(store.isReadOnly)
    }
}

// MARK: - Store: detail + tail

@MainActor
@Suite("JobsStore detail and event tail")
struct JobsStoreTailTests {
    /// Drive the store's tail exactly `count` poll iterations by ticking the
    /// manual clock and yielding so the loop body runs to its next sleep.
    private func advance(_ clock: ManualJobsClock, by count: Int) async {
        for _ in 0..<count {
            clock.tick()
            for _ in 0..<8 { await Task.yield() }
        }
    }

    @Test("open loads detail and the first poll folds the event snapshot")
    func openFoldsFirstSnapshot() async throws {
        let gateway = MockJobsGateway(detail: makeDetail(id: "j1", state: "running"))
        gateway.enqueueEventSnapshot(try decodeEvents(jobEventsFixture))
        let clock = ManualJobsClock()
        let store = JobsStore(gateway: gateway, clock: clock)

        await store.open(id: "j1")
        // Let the first poll iteration run (before its first sleep).
        for _ in 0..<8 { await Task.yield() }

        #expect(store.detail?.id == "j1")
        #expect(store.events.map(\.id) == [1, 2, 3, 4])
        #expect(store.tailError == nil)

        store.close()
    }

    @Test("a second poll appends only rows past the id cursor (no dupes)")
    func tailFoldsIncrementally() async throws {
        let first = Array(try decodeEvents(jobEventsFixture).prefix(2))  // ids 1,2 (still running)
        let full = try decodeEvents(jobEventsFixture)  // ids 1..4

        let gateway = MockJobsGateway()
        gateway.setDetailSequence([
            makeDetail(id: "j1", state: "running"),  // open()
            makeDetail(id: "j1", state: "running"),  // poll 1 detail refresh
            makeDetail(id: "j1", state: "running"),  // poll 2 detail refresh
        ])
        gateway.enqueueEventSnapshot(first)
        gateway.enqueueEventSnapshot(full)
        let clock = ManualJobsClock()
        let store = JobsStore(gateway: gateway, clock: clock)

        await store.open(id: "j1")
        for _ in 0..<8 { await Task.yield() }
        #expect(store.events.map(\.id) == [1, 2])

        // Second poll: the full snapshot re-includes 1,2 but only 3,4 are new.
        await advance(clock, by: 1)
        #expect(store.events.map(\.id) == [1, 2, 3, 4])

        store.close()
    }

    @Test("the tail stops once the job reaches a terminal state")
    func tailStopsOnTerminal() async throws {
        let gateway = MockJobsGateway()
        gateway.setDetailSequence([
            makeDetail(id: "j1", state: "running"),  // open()
            makeDetail(id: "j1", state: "completed"),  // poll 1 → terminal
        ])
        gateway.enqueueEventSnapshot(try decodeEvents(jobEventsFixture))
        let clock = ManualJobsClock()
        let store = JobsStore(gateway: gateway, clock: clock)

        await store.open(id: "j1")
        for _ in 0..<8 { await Task.yield() }

        // The loop should break on the terminal detail without ever sleeping.
        #expect(!store.isTailing)
        #expect(store.detail?.phase == .succeeded)
        // No sleep was requested because the loop broke before its first sleep.
        #expect(clock.recordedDelays.isEmpty)

        store.close()
    }

    @Test("a transient poll failure sets an error, backs off, and keeps tailing")
    func tailReconnectsAfterFailure() async throws {
        let gateway = MockJobsGateway()
        gateway.setDetailSequence([
            makeDetail(id: "j1", state: "running"),  // open()
            makeDetail(id: "j1", state: "running"),  // poll 2 detail refresh (after recovery)
        ])
        gateway.setThrowEventsOnce(true)  // first poll's jobEvents throws
        gateway.enqueueEventSnapshot(try decodeEvents(jobEventsFixture))
        let clock = ManualJobsClock()
        let store = JobsStore(gateway: gateway, clock: clock)

        await store.open(id: "j1")
        for _ in 0..<8 { await Task.yield() }

        // First poll failed: error set, no events yet, still tailing.
        #expect(store.tailError != nil)
        #expect(store.events.isEmpty)
        #expect(store.isTailing)
        // The failure backed off (attempt 1 → 4s, not the 2s steady interval).
        #expect(clock.recordedDelays.last == .seconds(4))

        // Recover on the next poll: error clears, events fold in.
        await advance(clock, by: 1)
        #expect(store.tailError == nil)
        #expect(store.events.map(\.id) == [1, 2, 3, 4])

        store.close()
    }

    @Test("opening a different job resets the cursor and clears the old tail")
    func openDifferentJobResets() async throws {
        let gateway = MockJobsGateway()
        gateway.setDetailSequence([
            makeDetail(id: "j1", state: "running"),
            makeDetail(id: "j1", state: "running"),
            makeDetail(id: "j2", state: "running"),
            makeDetail(id: "j2", state: "running"),
        ])
        gateway.enqueueEventSnapshot(try decodeEvents(jobEventsFixture))
        let clock = ManualJobsClock()
        let store = JobsStore(gateway: gateway, clock: clock)

        await store.open(id: "j1")
        for _ in 0..<8 { await Task.yield() }
        #expect(store.events.count == 4)

        // Different job: old events cleared, cursor reset (only the first two
        // events are available for j2 this time).
        gateway.resetEventSnapshots(Array(try decodeEvents(jobEventsFixture).prefix(2)))
        await store.open(id: "j2")
        for _ in 0..<8 { await Task.yield() }

        #expect(store.detail?.id == "j2")
        #expect(store.events.map(\.id) == [1, 2])

        store.close()
    }

    @Test("close stops the tail and clears open-job state")
    func closeClears() async throws {
        let gateway = MockJobsGateway(detail: makeDetail(id: "j1", state: "running"))
        gateway.enqueueEventSnapshot(try decodeEvents(jobEventsFixture))
        let store = JobsStore(gateway: gateway, clock: ManualJobsClock())

        await store.open(id: "j1")
        for _ in 0..<8 { await Task.yield() }
        #expect(store.detail != nil)

        store.close()
        #expect(store.detail == nil)
        #expect(store.events.isEmpty)
        #expect(!store.isTailing)
    }
}
