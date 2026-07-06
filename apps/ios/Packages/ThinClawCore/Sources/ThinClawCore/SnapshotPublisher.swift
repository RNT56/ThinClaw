import Foundation
import ThinClawSnapshotKit

/// The write side of the snapshot pipeline, abstracted so the publisher is
/// testable on macOS without an App Group entitlement and so production wraps a
/// `ThinClawSnapshotKit.SnapshotStore`.
///
/// Implementations persist the three snapshots to the shared container. They
/// need not be atomic across the set — each snapshot is independently versioned
/// and readers tolerate a momentarily-mixed set — but each individual write
/// should be atomic (the production `SnapshotStore` already is).
public protocol SnapshotSink: Sendable {
    func write(_ snapshots: ProjectedSnapshots) throws
}

/// A monotonic-enough time seam so the publisher's `generatedAt` stamps and
/// debounce timing are deterministic under test.
public protocol SnapshotClock: Sendable {
    /// Wall-clock time to stamp into `generatedAt`.
    func now() -> Date
    /// Suspend for `duration`, used by the debounce coalescer.
    func sleep(for duration: Duration) async throws
}

/// The system clock: `Date()` + `Task.sleep`.
public struct SystemSnapshotClock: SnapshotClock {
    public init() {}
    public func now() -> Date { Date() }
    public func sleep(for duration: Duration) async throws {
        try await Task.sleep(for: duration)
    }
}

/// Projects live agent state into the three App Group snapshots and writes them
/// through a ``SnapshotSink``, debouncing bursty updates so a flurry of SSE
/// events collapses into a single coalesced write.
///
/// ## Two entry points, one projection
/// - **Live (foreground):** ``ingest(event:threadTitle:)`` folds individual
///   `AgentEvent`s into the running agent-status state and schedules a debounced
///   publish. ``setApprovals(_:)`` / ``setJobs(_:)`` update the other two
///   sections the same way.
/// - **Snapshot (silent push / `BGAppRefresh`):** ``publishNow(_:)`` takes a
///   fully-formed ``SnapshotInputs`` (assembled from one-shot REST fetches) and
///   writes immediately, bypassing the debounce because the background wake
///   budget is short and there is nothing to coalesce.
///
/// ## Privacy
/// Every human-authored string routed through the publisher is truncated (and,
/// when previews are disabled, dropped) by the injected
/// ``SnapshotPrivacyPolicy`` before it reaches disk — snapshots are subject to
/// the same content-free discipline as pushes (docs/MOBILE_SECURITY.md, "Data
/// at rest").
///
/// UIKit-free and actor-isolated: the whole fold → debounce → write path runs
/// under `swift test` on macOS against a fake sink and a scripted clock.
public actor SnapshotPublisher {
    private let sink: any SnapshotSink
    private let clock: any SnapshotClock
    private let debounceInterval: Duration
    private var privacy: SnapshotPrivacyPolicy

    /// The running projection state, folded from live events and section
    /// setters.
    private var state: SnapshotInputs

    /// The in-flight debounce task, if a publish is pending.
    private var pendingPublish: Task<Void, Never>?

    /// The last inputs actually written, used to suppress no-op writes.
    private var lastWritten: SnapshotInputs?

    /// - Parameters:
    ///   - sink: Where projected snapshots are written.
    ///   - privacy: Truncation/preview policy applied to every string.
    ///   - clock: Time seam (defaults to the system clock).
    ///   - debounceInterval: How long to coalesce bursts before writing.
    ///     Defaults to 250 ms — long enough to swallow a token-by-token
    ///     `thinking`/`stream_chunk` flurry, short enough that a widget update
    ///     still feels prompt.
    public init(
        sink: any SnapshotSink,
        privacy: SnapshotPrivacyPolicy = .default,
        clock: any SnapshotClock = SystemSnapshotClock(),
        debounceInterval: Duration = .milliseconds(250)
    ) {
        self.sink = sink
        self.privacy = privacy
        self.clock = clock
        self.debounceInterval = debounceInterval
        self.state = SnapshotInputs()
    }

    // MARK: - Configuration

    /// Swap the preview/privacy policy (e.g. the operator toggled previews in
    /// Settings). The change takes effect on the next publish; callers usually
    /// follow with ``flush()`` to re-emit under the new policy immediately.
    public func setPrivacy(_ privacy: SnapshotPrivacyPolicy) {
        self.privacy = privacy
    }

    // MARK: - Live folding (foreground path)

    /// Fold one live SSE event into the agent-status projection and schedule a
    /// debounced publish. Events that carry no status-relevant signal
    /// (`heartbeat`, `unknown`, usage) are ignored.
    ///
    /// - Parameter threadTitle: The denormalized title of the event's thread,
    ///   when the caller knows it (from its thread cache). Passed alongside the
    ///   event because the event stream itself carries only a thread *id*.
    public func ingest(event: AgentEvent, threadTitle: String? = nil) {
        guard fold(event, threadTitle: threadTitle) else { return }
        scheduleDebouncedPublish()
    }

    /// Replace the pending-approvals section and schedule a publish. Wired to
    /// the ``ApprovalsStore`` so the widget's approval list tracks the app's.
    public func setApprovals(_ approvals: [ApprovalRequest]) {
        guard state.pendingApprovals != approvals else { return }
        state.pendingApprovals = approvals
        // A waiting-for-approval state is implied by a non-empty set; reflect it
        // in the status phase so the status widget agrees with the approvals
        // widget without a separate event.
        if !approvals.isEmpty, state.phase == .idle {
            state.phase = .waitingForApproval
        }
        scheduleDebouncedPublish()
    }

    /// Replace the jobs section and schedule a publish.
    public func setJobs(_ jobs: [SnapshotInputs.Job]) {
        guard state.jobs != jobs else { return }
        state.jobs = jobs
        scheduleDebouncedPublish()
    }

    /// Reset the unread counter (the user foregrounded / opened the active
    /// thread) and publish.
    public func clearUnread() {
        guard state.unreadCount != 0 else { return }
        state.unreadCount = 0
        scheduleDebouncedPublish()
    }

    /// Fold `event` into ``state``; returns whether the state changed (so the
    /// caller only schedules a write when something moved).
    private func fold(_ event: AgentEvent, threadTitle: String?) -> Bool {
        let before = state
        switch event {
        case .thinking(_, let thread):
            state.phase = .thinking
            adoptActiveThread(thread, title: threadTitle)
            state.activeToolName = nil
        case .streamChunk(_, let thread):
            state.phase = .streaming
            adoptActiveThread(thread, title: threadTitle)
            state.activeToolName = nil
        case .toolStarted(let name, let thread):
            state.phase = .runningTool
            state.activeToolName = name
            adoptActiveThread(thread, title: threadTitle)
        case .toolCompleted(_, _, let thread):
            // Tool finished: fall back to a generic "thinking" until the next
            // event; the tool name no longer applies.
            state.phase = .thinking
            state.activeToolName = nil
            adoptActiveThread(thread, title: threadTitle)
        case .approvalNeeded(let request):
            state.phase = .waitingForApproval
            adoptActiveThread(request.threadID, title: threadTitle)
        case .response(_, let thread):
            // Turn complete: the agent is idle, and a completed reply on a
            // thread other than nothing counts as one unread.
            state.phase = .idle
            state.activeToolName = nil
            adoptActiveThread(thread, title: threadTitle)
            state.unreadCount += 1
        case .error(_, let thread):
            state.phase = .error
            state.activeToolName = nil
            adoptActiveThread(thread, title: threadTitle)
        case .authRequired, .credentialPrompt, .usageUpdate, .heartbeat, .unknown:
            // No status-surface signal: leave the projection untouched.
            break
        }
        return state != before
    }

    /// Record the active thread id/title for the status snapshot, ignoring a
    /// nil id so a thread-less event does not clear a known active thread.
    private func adoptActiveThread(_ thread: ThreadID?, title: String?) {
        guard let thread else { return }
        state.activeThreadID = thread
        if let title { state.activeThreadTitle = title }
    }

    // MARK: - Immediate publish (background path)

    /// Write a fully-assembled ``SnapshotInputs`` immediately, bypassing the
    /// debounce. Used by the silent-push and `BGAppRefresh` handlers, which have
    /// already fetched a coherent snapshot over REST and run under a short wake
    /// budget. Also adopts the inputs as the new running state so a subsequent
    /// foreground fold continues from the freshest data.
    public func publishNow(_ inputs: SnapshotInputs) throws {
        cancelPending()
        state = inputs
        try write(inputs)
    }

    /// Flush any pending debounced state right now (e.g. on background, before
    /// the app is suspended) and cancel the timer.
    public func flush() throws {
        cancelPending()
        try write(state)
    }

    // MARK: - Debounce

    private func scheduleDebouncedPublish() {
        pendingPublish?.cancel()
        pendingPublish = Task { [debounceInterval] in
            do {
                try await clock.sleep(for: debounceInterval)
            } catch {
                return  // cancelled by a newer edit; that task will publish.
            }
            guard !Task.isCancelled else { return }
            self.publishPending()
        }
    }

    private func publishPending() {
        pendingPublish = nil
        try? write(state)
    }

    private func cancelPending() {
        pendingPublish?.cancel()
        pendingPublish = nil
    }

    /// Project + write, suppressing a write when the inputs are byte-identical
    /// to the last one that reached disk (avoids waking widget timelines for a
    /// no-op).
    private func write(_ inputs: SnapshotInputs) throws {
        if let lastWritten, lastWritten == inputs { return }
        let snapshots = inputs.project(at: clock.now(), privacy: privacy)
        try sink.write(snapshots)
        lastWritten = inputs
    }
}
