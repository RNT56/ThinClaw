import Foundation
import ThinClawAPI
import ThinClawCore
import ThinClawSnapshotKit
import ThinClawWidgetKitShared

/// Owns the App Group snapshot pipeline on the app side: a ``SnapshotPublisher``
/// writing into the shared container, plus the two ways state reaches it.
///
/// - **Live (foreground):** ``subscribeToApprovals(_:)`` folds the shared
///   ``ApprovalsStore``'s pending set into the publisher as it changes, so the
///   approvals widget tracks the app without a network round-trip.
/// - **Fetch (silent push / `BGAppRefresh` / foreground kick):** ``refresh()``
///   pulls gateway status + pending approvals + the jobs list over the pinned
///   REST client, projects them into a ``SnapshotInputs``, and writes it
///   immediately (`publishNow`), then the caller reloads widget timelines.
///
/// Content-free discipline (docs/MOBILE_SECURITY.md, "Data at rest"): every
/// human-authored string is truncated/dropped by the injected
/// ``SnapshotPrivacyPolicy`` before it reaches disk. The pipeline degrades
/// gracefully — with no App Group container (missing entitlement / test host)
/// the publisher is `nil` and every call is a no-op.
@MainActor
final class SnapshotService {
    private let publisher: SnapshotPublisher?

    /// Build the service over the shared App Group container. `publisher` is
    /// `nil` when the container is unavailable, so callers need not special-case
    /// the unpaired/entitlement-missing path.
    init(privacy: SnapshotPrivacyPolicy = .default) {
        if let sink = SnapshotStoreSink(appGroupID: WidgetSnapshotAccess.appGroupID) {
            self.publisher = SnapshotPublisher(sink: sink, privacy: privacy)
        } else {
            self.publisher = nil
        }
    }

    /// Fold the approvals store's pending set into the publisher whenever it
    /// changes. Runs until the returned task is cancelled (on unpair/teardown).
    /// The status phase is nudged to `waitingForApproval` by the publisher when
    /// the set is non-empty, so the status widget and approvals widget agree.
    func mirror(approvals: [ApprovalRequest]) async {
        await publisher?.setApprovals(approvals)
    }

    /// Fold a single live event into the running agent-status projection.
    func ingest(event: AgentEvent, threadTitle: String? = nil) async {
        await publisher?.ingest(event: event, threadTitle: threadTitle)
    }

    /// Fetch a coherent snapshot over `client` and write it immediately.
    /// Best-effort per section: a failing endpoint contributes an empty section
    /// rather than aborting the whole refresh, so a partial gateway outage still
    /// updates the sections that answered. Returns whether anything was written
    /// (always `true` when a publisher exists — a background wake that produced
    /// a fresh snapshot counts as `.newData`).
    @discardableResult
    func refresh(client: Client) async -> Bool {
        guard let publisher else { return false }
        let inputs = await Self.fetchInputs(client: client)
        try? await publisher.publishNow(inputs)
        return true
    }

    /// Assemble ``SnapshotInputs`` from the three read endpoints. Each fetch is
    /// independent and failure-tolerant.
    private static func fetchInputs(client: Client) async -> SnapshotInputs {
        async let statusPhase = fetchStatusPhase(client: client)
        async let approvals = fetchApprovals(client: client)
        async let jobs = fetchJobs(client: client)

        let pending = await approvals
        var inputs = SnapshotInputs(
            phase: await statusPhase,
            pendingApprovals: pending,
            jobs: await jobs)
        // A pending approval outranks an "idle" status phase so the surfaces
        // stay consistent even if the status endpoint reports idle.
        if !pending.isEmpty, inputs.phase == .idle {
            inputs.phase = .waitingForApproval
        }
        return inputs
    }

    private static func fetchStatusPhase(client: Client) async -> AgentStatusSnapshot.Phase {
        // The gateway status endpoint reports gateway-wide health, not a single
        // run's phase; there is no per-run "thinking/streaming" signal on the
        // cold path. Treat a reachable gateway as `idle` (no active run implied)
        // and an unreachable one as `error`.
        guard
            let output = try? await client.gatewayStatusHandler(),
            case .ok = output
        else {
            return .error
        }
        return .idle
    }

    private static func fetchApprovals(client: Client) async -> [ApprovalRequest] {
        guard
            let output = try? await client.chatApprovalsHandler(),
            let body = try? output.ok.body.json
        else {
            return []
        }
        return body.approvals.map { entry in
            ApprovalRequest(
                requestID: entry.requestId,
                toolName: entry.toolName,
                description: entry.description,
                parameters: entry.parameters,
                risk: RiskTier(wire: entry.risk.rawValue),
                threadID: entry.threadId.map(ThreadID.init))
        }
    }

    private static func fetchJobs(client: Client) async -> [SnapshotInputs.Job] {
        guard
            let output = try? await client.jobsListHandler(),
            let body = try? output.ok.body.json
        else {
            return []
        }
        return body.jobs.map { job in
            SnapshotInputs.Job(
                id: job.id,
                title: job.title,
                phase: Self.jobPhase(from: job.state),
                startedAt: Self.parseTimestamp(job.startedAt ?? job.createdAt))
        }
    }

    /// Map a gateway job `state` string onto the snapshot's coarse phase,
    /// mirroring the gateway's own bucketing
    /// (`thinclaw_gateway::web::jobs::summary_bucket`).
    static func jobPhase(from state: String) -> JobsSnapshot.JobSummary.Phase {
        switch state {
        case "pending", "queued", "creating":
            return .queued
        case "in_progress", "running", "submitted", "accepted":
            return .running
        case "completed", "succeeded":
            return .succeeded
        case "failed", "abandoned", "cancelled", "interrupted", "stuck":
            return .failed
        default:
            // Unknown states are shown as running rather than dropped: a job the
            // gateway still lists is more likely active than terminal.
            return .running
        }
    }

    /// Parse an RFC-3339 timestamp, falling back to the epoch so a malformed
    /// value never drops the row.
    static func parseTimestamp(_ raw: String) -> Date {
        Self.iso8601.date(from: raw)
            ?? Self.iso8601NoFraction.date(from: raw)
            ?? Date(timeIntervalSince1970: 0)
    }

    private static let iso8601: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f
    }()

    private static let iso8601NoFraction: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f
    }()
}
