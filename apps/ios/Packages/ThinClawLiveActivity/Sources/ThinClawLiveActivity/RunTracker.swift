import Foundation
import ThinClawCore

/// The content-free run phase the Live Activity renders. A Foundation-only
/// mirror of `AgentRunAttributes.ContentState.RunPhase` (which is iOS-only,
/// behind `canImport(ActivityKit)`), so the reducer stays macOS-testable. The
/// raw values match the wire/attributes enum exactly, so mapping across the
/// boundary is lossless.
public enum RunPhase: String, Hashable, Sendable {
    case thinking
    case runningTool
    case awaitingApproval
    case done
    case failed
}

extension RunPhase {
    /// Whether this phase is terminal — a run in this phase has finished and the
    /// activity should end.
    public var isTerminal: Bool {
        switch self {
        case .thinking, .runningTool, .awaitingApproval:
            return false
        case .done, .failed:
            return true
        }
    }
}

/// The content-free state the reducer tracks for one run. Carries only a phase,
/// optional tool name, optional progress, optional pending-approval id, and the
/// monotonic revision (docs/MOBILE_SECURITY.md D-N2 — no prompt text, no tool
/// arguments). `toolName`/`pendingApprovalID` are populated only from *local*
/// SSE updates, never from a push, and so never transit APNs.
public struct RunState: Hashable, Sendable {
    public var phase: RunPhase
    public var toolName: String?
    public var progress: Int?
    public var pendingApprovalID: String?
    public var revision: UInt64

    public init(
        phase: RunPhase,
        toolName: String? = nil,
        progress: Int? = nil,
        pendingApprovalID: String? = nil,
        revision: UInt64
    ) {
        self.phase = phase
        self.toolName = toolName
        self.progress = progress
        self.pendingApprovalID = pendingApprovalID
        self.revision = revision
    }
}

/// An input to the ``RunTracker`` reducer. These are the *already-classified*
/// signals for the tracked thread; `LiveActivityManager` derives them from the
/// raw `AgentEvent` stream (and from ActivityKit push callbacks) before feeding
/// them in, so the reducer has no ActivityKit or transport dependency.
public enum RunInput: Hashable, Sendable {
    /// A run began (first `thinking`/`tool_started`/status for the thread).
    case runStarted(threadID: ThreadID, threadTitle: String)
    /// The agent is thinking / between tools.
    case thinking(threadID: ThreadID)
    /// A tool started running (local-only tool name; never pushed).
    case toolStarted(threadID: ThreadID, toolName: String?)
    /// A tool reported fractional progress in [0, 100].
    case progress(threadID: ThreadID, percent: Int)
    /// The agent is blocked on an operator approval.
    case awaitingApproval(threadID: ThreadID, requestID: String?)
    /// The run completed successfully (a `response` closing the turn).
    case completed(threadID: ThreadID)
    /// The run failed (an `error` for the thread).
    case failed(threadID: ThreadID)
    /// A late inbound *push* revision landed. Used only to keep the reducer's
    /// revision counter ahead of any push, so a subsequent local update always
    /// supersedes it. Never regresses local state.
    case pushRevisionObserved(threadID: ThreadID, revision: UInt64)
}

/// The side effect the reducer decides on. `LiveActivityManager` performs these
/// against ActivityKit + the gateway; the reducer itself is pure.
public enum RunAction: Hashable, Sendable {
    /// Request a new Activity for the thread with this initial state.
    case start(threadID: ThreadID, threadTitle: String, state: RunState)
    /// Update the existing Activity to this state (local update — lower latency
    /// than a push, and it carries the monotonic revision so a late push can be
    /// dropped by the widget).
    case update(threadID: ThreadID, state: RunState)
    /// End the Activity for the thread with this final state.
    case end(threadID: ThreadID, state: RunState)
}

/// A pure reducer that decides start/update/end + a monotonically increasing
/// revision for at most one active run per thread, from a sequence of
/// ``RunInput``s.
///
/// Invariants it guarantees (the tested contract):
/// - **Start once.** A `runStarted` (or any run signal) for a thread with no
///   active run emits exactly one `.start`; further signals emit `.update`.
///   A duplicate `runStarted` for an already-active run does not restart it.
/// - **Monotonic revision.** Every emitted state's `revision` is strictly
///   greater than the previous one for that thread, and greater than any
///   observed push revision — so a late push (which carries an older revision)
///   never regresses the widget, which keeps the highest revision it has seen.
/// - **End on completion.** `completed`/`failed` emits `.end` and clears the
///   run so a later signal for the thread starts a fresh activity.
///
/// Not thread-safe by itself; ``LiveActivityManager`` owns it on the main actor.
public struct RunTracker {
    /// Per-thread live run bookkeeping.
    private struct Run {
        var threadTitle: String
        var state: RunState
    }

    private var runs: [ThreadID: Run] = [:]

    public init() {}

    /// Whether a run is currently tracked for `thread`.
    public func isTracking(_ thread: ThreadID) -> Bool {
        runs[thread] != nil
    }

    /// The current tracked state for `thread`, if any (mostly for tests).
    public func state(for thread: ThreadID) -> RunState? {
        runs[thread]?.state
    }

    /// Feed one input; return the action to perform, or `nil` if the input does
    /// not change anything actionable (e.g. progress for an unknown thread, or a
    /// push-revision observation that is already behind the local counter).
    public mutating func reduce(_ input: RunInput) -> RunAction? {
        switch input {
        case let .runStarted(threadID, threadTitle):
            return startIfNeeded(threadID, threadTitle: threadTitle)

        case let .thinking(threadID):
            return applyPhase(threadID, phase: .thinking, toolName: nil, clearApproval: true)

        case let .toolStarted(threadID, toolName):
            return applyPhase(
                threadID, phase: .runningTool, toolName: toolName, clearApproval: true)

        case let .progress(threadID, percent):
            return applyProgress(threadID, percent: percent)

        case let .awaitingApproval(threadID, requestID):
            return applyPhase(
                threadID, phase: .awaitingApproval, toolName: nil,
                pendingApprovalID: requestID, clearApproval: false)

        case let .completed(threadID):
            return end(threadID, phase: .done)

        case let .failed(threadID):
            return end(threadID, phase: .failed)

        case let .pushRevisionObserved(threadID, revision):
            observePushRevision(threadID, revision: revision)
            return nil
        }
    }

    // MARK: - Reducers

    private mutating func startIfNeeded(_ thread: ThreadID, threadTitle: String) -> RunAction? {
        // Start once: a run already tracked for this thread is not restarted.
        if runs[thread] != nil { return nil }
        let state = RunState(phase: .thinking, revision: 1)
        runs[thread] = Run(threadTitle: threadTitle, state: state)
        return .start(threadID: thread, threadTitle: threadTitle, state: state)
    }

    private mutating func applyPhase(
        _ thread: ThreadID,
        phase: RunPhase,
        toolName: String?,
        pendingApprovalID: String? = nil,
        clearApproval: Bool
    ) -> RunAction? {
        // A progress/tool/approval signal for a thread we are not yet tracking
        // implicitly starts the run (some runs open with `tool_started`, not a
        // bare status), so the activity still appears.
        guard var run = runs[thread] else {
            let state = RunState(
                phase: phase, toolName: toolName, pendingApprovalID: pendingApprovalID,
                revision: 1)
            runs[thread] = Run(threadTitle: thread.rawValue, state: state)
            return .start(threadID: thread, threadTitle: thread.rawValue, state: state)
        }
        run.state.phase = phase
        run.state.toolName = toolName
        if clearApproval {
            run.state.pendingApprovalID = nil
        } else {
            run.state.pendingApprovalID = pendingApprovalID
        }
        run.state.revision = nextRevision(after: run.state.revision)
        runs[thread] = run
        return .update(threadID: thread, state: run.state)
    }

    private mutating func applyProgress(_ thread: ThreadID, percent: Int) -> RunAction? {
        guard var run = runs[thread] else { return nil }
        run.state.progress = clampProgress(percent)
        run.state.revision = nextRevision(after: run.state.revision)
        runs[thread] = run
        return .update(threadID: thread, state: run.state)
    }

    private mutating func end(_ thread: ThreadID, phase: RunPhase) -> RunAction? {
        // Ending a thread we never started is a no-op (e.g. a stray `response`
        // for a thread whose run we never tracked).
        guard var run = runs.removeValue(forKey: thread) else { return nil }
        run.state.phase = phase
        run.state.toolName = nil
        run.state.pendingApprovalID = nil
        run.state.revision = nextRevision(after: run.state.revision)
        return .end(threadID: thread, state: run.state)
    }

    /// Bump the local revision counter to stay ahead of a late push. Never emits
    /// an action — the widget already applied the push; this only ensures the
    /// *next* local update outranks it so state does not flip backwards.
    private mutating func observePushRevision(_ thread: ThreadID, revision: UInt64) {
        guard var run = runs[thread] else { return }
        if revision >= run.state.revision {
            run.state.revision = revision
            runs[thread] = run
        }
    }

    // MARK: - Helpers

    /// The next revision, saturating at `UInt64.max` rather than overflowing.
    /// A run producing 2^64 updates is not reachable in practice; saturating
    /// keeps monotonicity total instead of trapping.
    private func nextRevision(after current: UInt64) -> UInt64 {
        current == UInt64.max ? UInt64.max : current + 1
    }

    private func clampProgress(_ percent: Int) -> Int {
        min(100, max(0, percent))
    }
}
