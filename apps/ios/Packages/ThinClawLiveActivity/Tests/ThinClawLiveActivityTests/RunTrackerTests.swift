import Foundation
import Testing
import ThinClawCore

@testable import ThinClawLiveActivity

/// The tested contract for the pure run reducer: start-once, monotonic
/// revision, end-on-completion, and local-vs-push revision reconciliation.
/// ActivityKit is never touched here — this runs on a Mac host under
/// `swift test`.
@Suite("RunTracker")
struct RunTrackerTests {
    private let thread = ThreadID("web-1")

    // MARK: - Start once

    @Test("first run signal emits exactly one .start at revision 1")
    func startsOnce() {
        var tracker = RunTracker()
        let action = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat"))
        guard case let .start(t, title, state) = action else {
            Issue.record("expected .start, got \(String(describing: action))")
            return
        }
        #expect(t == thread)
        #expect(title == "Chat")
        #expect(state.phase == .thinking)
        #expect(state.revision == 1)
        #expect(tracker.isTracking(thread))
    }

    @Test("a duplicate runStarted for an active run does not restart it")
    func doesNotRestart() {
        var tracker = RunTracker()
        _ = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat"))
        let second = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat again"))
        #expect(second == nil)
    }

    @Test("a tool_started for an untracked thread starts the run")
    func toolStartedImplicitlyStarts() {
        var tracker = RunTracker()
        let action = tracker.reduce(.toolStarted(threadID: thread, toolName: "read_file"))
        guard case let .start(_, _, state) = action else {
            Issue.record("expected .start, got \(String(describing: action))")
            return
        }
        #expect(state.phase == .runningTool)
        #expect(state.toolName == "read_file")
        #expect(state.revision == 1)
    }

    // MARK: - Monotonic revision

    @Test("every emitted state strictly increases the revision")
    func revisionMonotonic() {
        var tracker = RunTracker()
        var last: UInt64 = 0
        let inputs: [RunInput] = [
            .runStarted(threadID: thread, threadTitle: "Chat"),
            .toolStarted(threadID: thread, toolName: "grep"),
            .progress(threadID: thread, percent: 10),
            .progress(threadID: thread, percent: 40),
            .thinking(threadID: thread),
            .awaitingApproval(threadID: thread, requestID: "req-1"),
            .completed(threadID: thread),
        ]
        for input in inputs {
            guard let action = tracker.reduce(input) else { continue }
            let revision = action.state.revision
            #expect(revision > last, "revision \(revision) did not exceed \(last)")
            last = revision
        }
        #expect(last == 7)
    }

    @Test("progress is clamped to 0...100")
    func progressClamped() {
        var tracker = RunTracker()
        _ = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat"))
        _ = tracker.reduce(.progress(threadID: thread, percent: 250))
        #expect(tracker.state(for: thread)?.progress == 100)
        _ = tracker.reduce(.progress(threadID: thread, percent: -5))
        #expect(tracker.state(for: thread)?.progress == 0)
    }

    // MARK: - End on completion

    @Test("completion emits .end at .done and forgets the run")
    func endsOnCompletion() {
        var tracker = RunTracker()
        _ = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat"))
        let action = tracker.reduce(.completed(threadID: thread))
        guard case let .end(_, state) = action else {
            Issue.record("expected .end, got \(String(describing: action))")
            return
        }
        #expect(state.phase == .done)
        #expect(!tracker.isTracking(thread))
    }

    @Test("failure emits .end at .failed")
    func endsOnFailure() {
        var tracker = RunTracker()
        _ = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat"))
        let action = tracker.reduce(.failed(threadID: thread))
        guard case let .end(_, state) = action else {
            Issue.record("expected .end, got \(String(describing: action))")
            return
        }
        #expect(state.phase == .failed)
        #expect(!tracker.isTracking(thread))
    }

    @Test("ending an untracked thread is a no-op")
    func endUntrackedNoop() {
        var tracker = RunTracker()
        #expect(tracker.reduce(.completed(threadID: thread)) == nil)
    }

    @Test("after an end, a new signal starts a fresh activity at revision 1")
    func restartsAfterEnd() {
        var tracker = RunTracker()
        _ = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat"))
        _ = tracker.reduce(.completed(threadID: thread))
        let action = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat 2"))
        guard case let .start(_, _, state) = action else {
            Issue.record("expected .start, got \(String(describing: action))")
            return
        }
        #expect(state.revision == 1)
    }

    // MARK: - Local-vs-push reconciliation

    @Test("a late push revision keeps the next local update ahead of it")
    func pushRevisionReconciliation() {
        var tracker = RunTracker()
        _ = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat"))
        // Local counter is at 1. A push carrying revision 9 lands (the gateway
        // pushed several updates the app did not locally apply).
        _ = tracker.reduce(.pushRevisionObserved(threadID: thread, revision: 9))
        #expect(tracker.state(for: thread)?.revision == 9)
        // The next local update must outrank the observed push (>= 10), so the
        // widget — which keeps the highest revision — applies the local state.
        let action = tracker.reduce(.toolStarted(threadID: thread, toolName: "x"))
        #expect(action?.state.revision == 10)
    }

    @Test("a stale push revision never regresses the local counter")
    func stalePushIgnored() {
        var tracker = RunTracker()
        _ = tracker.reduce(.runStarted(threadID: thread, threadTitle: "Chat"))
        _ = tracker.reduce(.toolStarted(threadID: thread, toolName: "a"))  // rev 2
        _ = tracker.reduce(.thinking(threadID: thread))  // rev 3
        // A push carrying an older revision (1) must not drag the counter back.
        _ = tracker.reduce(.pushRevisionObserved(threadID: thread, revision: 1))
        #expect(tracker.state(for: thread)?.revision == 3)
        let action = tracker.reduce(.progress(threadID: thread, percent: 50))
        #expect(action?.state.revision == 4)
    }

    @Test("pushRevisionObserved for an untracked thread is a no-op")
    func pushRevisionUntracked() {
        var tracker = RunTracker()
        #expect(tracker.reduce(.pushRevisionObserved(threadID: thread, revision: 5)) == nil)
        #expect(!tracker.isTracking(thread))
    }

    // MARK: - Isolation between threads

    @Test("each thread keeps its own run and revision")
    func perThreadIsolation() {
        var tracker = RunTracker()
        let other = ThreadID("web-2")
        _ = tracker.reduce(.runStarted(threadID: thread, threadTitle: "A"))
        _ = tracker.reduce(.toolStarted(threadID: thread, toolName: "t"))  // thread rev 2
        _ = tracker.reduce(.runStarted(threadID: other, threadTitle: "B"))  // other rev 1
        #expect(tracker.state(for: thread)?.revision == 2)
        #expect(tracker.state(for: other)?.revision == 1)
        #expect(tracker.isTracking(thread))
        #expect(tracker.isTracking(other))
    }
}

/// Small accessor so the tests can read a revision off any emitted action
/// without re-switching each time.
extension RunAction {
    fileprivate var state: RunState {
        switch self {
        case let .start(_, _, state), let .update(_, state), let .end(_, state):
            return state
        }
    }
}
