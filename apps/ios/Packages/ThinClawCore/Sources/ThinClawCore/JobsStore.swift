import Foundation
import Observation

/// The read-only network surface the Jobs glance needs, abstracted so the store
/// runs on macOS under `swift test` with no live gateway. The production
/// adapter (`FeatureJobs.GatewayJobsAdapter`) wraps the generated REST client
/// for list/summary/detail and a hand-rolled pinned fetch for the event tail.
///
/// Every method is read-only by construction: the phone token holds `jobs:read`
/// only, so there is deliberately **no** cancel/restart/prompt method on this
/// protocol (those endpoints are `POST` and not device-scoped — see
/// `docs/MOBILE_SECURITY.md`). The glance cannot mutate a job.
public protocol JobsGateway: Sendable {
    /// `GET /api/jobs` — the current job list, newest-first.
    func listJobs() async throws -> [Job]
    /// `GET /api/jobs/summary` — aggregate counts.
    func jobsSummary() async throws -> JobsSummary
    /// `GET /api/jobs/{id}` — full detail incl. transitions.
    func jobDetail(id: String) async throws -> JobDetail
    /// `GET /api/jobs/{id}/events` — a **snapshot** of the job's stored event
    /// log (not a stream). The store polls this and folds new rows by id.
    func jobEvents(id: String) async throws -> [JobEvent]
}

/// The sleep seam for the event-tail poll loop, so tests advance time by hand
/// instead of waiting on a wall clock. Production uses ``SystemJobsClock``.
public protocol JobsClock: Sendable {
    /// Suspend for `duration`, honoring cancellation.
    func sleep(for duration: Duration) async throws
}

/// Wall-clock ``JobsClock``.
public struct SystemJobsClock: JobsClock {
    public init() {}
    public func sleep(for duration: Duration) async throws {
        try await Task.sleep(for: duration)
    }
}

/// Poll cadence + backoff for the per-job event tail.
///
/// Because `/api/jobs/{id}/events` is a JSON snapshot (there is no SSE stream
/// for jobs on the gateway), the "live" tail is a bounded poll. Successful
/// polls use ``interval``; consecutive failures back off geometrically up to
/// ``maxInterval`` so a flaky link does not hammer the gateway.
public struct JobsPollPolicy: Hashable, Sendable {
    public var interval: Duration
    public var maxInterval: Duration
    public var multiplier: Double

    public static let `default` = JobsPollPolicy(
        interval: .seconds(2),
        maxInterval: .seconds(30),
        multiplier: 2)

    public init(interval: Duration, maxInterval: Duration, multiplier: Double = 2) {
        self.interval = interval
        self.maxInterval = maxInterval
        self.multiplier = multiplier
    }

    /// Delay before the next poll given the count of consecutive failures since
    /// the last success (0 → the steady-state ``interval``).
    public func delay(consecutiveFailures: Int) -> Duration {
        guard consecutiveFailures > 0 else { return interval }
        let base = interval.seconds
        let capped = min(maxInterval.seconds, base * pow(multiplier, Double(consecutiveFailures)))
        return .seconds(capped)
    }
}

/// Drives the read-only Jobs glance: lists jobs, loads a job's detail, and
/// tails a job's event log by polling. UI-free by design (no SwiftUI / design
/// imports) so the list/detail mapping, the id-cursor fold, and the poll
/// backoff are all exercised by plain `swift test` on macOS with a mocked
/// gateway and a manual clock. `FeatureJobs` supplies the SwiftUI screens and
/// the concrete ``JobsGateway`` adapter.
///
/// Read-only: there is no method here that mutates a job. The store exposes a
/// stable ``isReadOnly`` flag the UI reads to render its "view only" affordance
/// truthfully rather than as decoration.
@MainActor
@Observable
public final class JobsStore {
    /// The job list, newest-first as returned by the gateway.
    public private(set) var jobs: [Job] = []
    /// Aggregate counts; nil until first loaded.
    public private(set) var summary: JobsSummary?
    /// True while a list refresh is in flight (drives the initial spinner; the
    /// pull-to-refresh control shows its own indicator).
    public private(set) var isLoadingList = false
    /// Last list-load error message, if the most recent refresh failed.
    public private(set) var listError: String?

    /// Detail for the currently-open job, if any.
    public private(set) var detail: JobDetail?
    /// The folded event tail for the open job, oldest-first.
    public private(set) var events: [JobEvent] = []
    /// True while the per-job tail poll loop is running.
    public private(set) var isTailing = false
    /// Last tail error message, if the most recent poll failed (cleared on the
    /// next success).
    public private(set) var tailError: String?

    /// The phone can never mutate a job from here (`jobs:read` only). Constant.
    public let isReadOnly = true

    private let gateway: any JobsGateway
    private let clock: any JobsClock
    private let pollPolicy: JobsPollPolicy

    /// Highest event id already folded for the open job — the append-only poll
    /// cursor. Reset whenever a different job is opened.
    private var lastEventID: Int64?
    private var tailTask: Task<Void, Never>?

    public init(
        gateway: any JobsGateway,
        clock: any JobsClock = SystemJobsClock(),
        pollPolicy: JobsPollPolicy = .default
    ) {
        self.gateway = gateway
        self.clock = clock
        self.pollPolicy = pollPolicy
    }

    // MARK: - List

    /// Refresh the job list and summary (initial load + pull-to-refresh).
    /// Best-effort per-call: a summary failure does not blank an otherwise-good
    /// list, and vice versa; the list is authoritative and replaces wholesale
    /// (unlike approvals, the jobs list is a complete server snapshot).
    public func refresh() async {
        isLoadingList = true
        defer { isLoadingList = false }

        do {
            jobs = try await gateway.listJobs()
            listError = nil
        } catch is CancellationError {
            // Leave the last-known list in place on cancellation.
        } catch {
            listError = Self.message(for: error)
        }

        // Summary is a nice-to-have header; failure must not clobber the list.
        if let summary = try? await gateway.jobsSummary() {
            self.summary = summary
        }
    }

    // MARK: - Detail + tail

    /// Open `id`: load its detail and start the live event tail. Idempotent for
    /// the same id (a repeat call re-loads detail and leaves the running tail
    /// alone); opening a *different* id first tears down the previous tail.
    public func open(id: String) async {
        if detail?.id != id {
            stopTail()
            detail = nil
            events = []
            lastEventID = nil
            tailError = nil
        }

        do {
            detail = try await gateway.jobDetail(id: id)
        } catch is CancellationError {
            return
        } catch {
            tailError = Self.message(for: error)
        }

        startTail(id: id)
    }

    /// Stop the tail and clear the open-job state (detail view dismissed).
    public func close() {
        stopTail()
        detail = nil
        events = []
        lastEventID = nil
        tailError = nil
    }

    private func startTail(id: String) {
        guard tailTask == nil else { return }
        isTailing = true
        tailTask = Task { [weak self] in
            await self?.runTail(id: id)
        }
    }

    private func stopTail() {
        tailTask?.cancel()
        tailTask = nil
        isTailing = false
    }

    /// The poll loop: fetch the event snapshot, fold new rows, refresh detail so
    /// the state/transitions track, and stop once the job is terminal. Backs off
    /// on failure per ``pollPolicy`` and reconnects (keeps polling) on transient
    /// errors — a single failed poll never tears the tail down.
    private func runTail(id: String) async {
        var consecutiveFailures = 0

        while !Task.isCancelled {
            do {
                let fetched = try await gateway.jobEvents(id: id)
                fold(fetched)
                tailError = nil
                consecutiveFailures = 0

                // Track the job's own lifecycle from detail so the tail stops on
                // a terminal state instead of polling a finished job forever.
                if let refreshed = try? await gateway.jobDetail(id: id) {
                    detail = refreshed
                    if refreshed.phase.isTerminal {
                        // One final fold already happened above; end the tail.
                        break
                    }
                }
            } catch is CancellationError {
                break
            } catch {
                consecutiveFailures += 1
                tailError = Self.message(for: error)
            }

            do {
                try await clock.sleep(for: pollPolicy.delay(consecutiveFailures: consecutiveFailures))
            } catch {
                break  // cancelled during sleep
            }
        }

        isTailing = false
    }

    /// Fold a freshly-fetched snapshot into ``events`` by monotonic id: append
    /// only rows past ``lastEventID``. The gateway returns the full log each
    /// poll, so this de-dupes to the append-only delta and keeps the tail stable
    /// (no reordering or flicker of already-shown rows).
    private func fold(_ fetched: [JobEvent]) {
        let cursor = lastEventID
        let fresh =
            fetched
            .filter { cursor == nil || $0.id > cursor! }
            .sorted { $0.id < $1.id }
        guard !fresh.isEmpty else { return }
        events.append(contentsOf: fresh)
        lastEventID = events.map(\.id).max()
    }

    // MARK: - Helpers

    private static func message(for error: any Error) -> String {
        (error as? LocalizedError)?.errorDescription ?? "\(error)"
    }
}

extension Duration {
    /// This duration as a fractional number of seconds.
    fileprivate var seconds: Double {
        let components = self.components
        return Double(components.seconds) + Double(components.attoseconds) / 1e18
    }
}
