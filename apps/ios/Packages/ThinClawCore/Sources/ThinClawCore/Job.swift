import Foundation

/// A background job as surfaced to the read-only mobile Jobs glance.
///
/// The phone token holds only `jobs:read`, so this is a **view**: no cancel,
/// restart, or prompt affordance is representable here (those endpoints are
/// `POST` and not device-scoped — see `docs/MOBILE_SECURITY.md`). The store and
/// UI treat every job as observe-only.
public struct Job: Identifiable, Hashable, Sendable {
    /// The job UUID (opaque string; the gateway returns a stringified UUID).
    public let id: String
    /// Operator-facing title.
    public var title: String
    /// The gateway's raw state string (e.g. `running`, `completed`, `failed`).
    /// Preserved verbatim for display; ``phase`` is the normalized bucket.
    public var state: String
    /// Normalized lifecycle bucket derived from ``state`` for iconography and
    /// grouping. Unknown states fall into ``JobPhase/unknown``.
    public var phase: JobPhase
    /// Creation time, parsed from the gateway's RFC3339 `created_at`. Nil if the
    /// timestamp could not be parsed.
    public var createdAt: Date?
    /// Start time, parsed from the gateway's RFC3339 `started_at`. Nil when the
    /// job has not started or the timestamp could not be parsed.
    public var startedAt: Date?

    public init(
        id: String,
        title: String,
        state: String,
        phase: JobPhase,
        createdAt: Date?,
        startedAt: Date?
    ) {
        self.id = id
        self.title = title
        self.state = state
        self.phase = phase
        self.createdAt = createdAt
        self.startedAt = startedAt
    }
}

/// Normalized job lifecycle bucket.
///
/// The gateway emits several backend-specific state strings (direct-worker vs
/// sandbox); ``JobPhase/from(state:)`` collapses them into the small set the
/// glance renders, mirroring the gateway's own summary bucketing
/// (`crates/thinclaw-gateway/src/web/jobs.rs`).
public enum JobPhase: String, Hashable, Sendable, CaseIterable {
    case pending
    case running
    case succeeded
    case failed
    case cancelled
    case stuck
    case unknown

    /// Whether the job has reached a terminal state (no more transitions
    /// expected). The store uses this to stop the live event tail once a job
    /// finishes so it does not poll a dead job forever.
    public var isTerminal: Bool {
        switch self {
        case .succeeded, .failed, .cancelled, .stuck:
            return true
        case .pending, .running, .unknown:
            return false
        }
    }

    /// Map a raw gateway state/status string to a bucket. Covers both the
    /// direct-worker states and the sandbox statuses the gateway can return;
    /// anything unrecognized becomes ``unknown`` (fail-open to *display*, never
    /// to a false "done").
    public static func from(state: String) -> JobPhase {
        switch state.lowercased() {
        case "pending", "queued", "creating":
            return .pending
        case "in_progress", "running":
            return .running
        case "completed", "succeeded", "submitted", "accepted":
            return .succeeded
        case "failed", "abandoned", "interrupted":
            return .failed
        case "cancelled", "canceled":
            return .cancelled
        case "stuck":
            return .stuck
        default:
            return .unknown
        }
    }
}

/// A single state transition in a job's history, as returned by
/// `GET /api/jobs/{id}` (`TransitionInfo`).
public struct JobTransition: Hashable, Sendable, Identifiable {
    /// Stable identity for list diffing: the transition is uniquely keyed by its
    /// timestamp + target state within one job.
    public var id: String { "\(timestamp?.timeIntervalSince1970 ?? 0)-\(to)" }

    public var from: String
    public var to: String
    public var reason: String?
    /// Parsed transition time; nil if the RFC3339 stamp could not be parsed.
    public var timestamp: Date?

    public init(from: String, to: String, reason: String?, timestamp: Date?) {
        self.from = from
        self.to = to
        self.reason = reason
        self.timestamp = timestamp
    }
}

/// Full detail for one job, backing the detail view.
public struct JobDetail: Identifiable, Hashable, Sendable {
    public let id: String
    public var title: String
    public var description: String
    public var state: String
    public var phase: JobPhase
    public var createdAt: Date?
    public var startedAt: Date?
    public var completedAt: Date?
    /// Elapsed wall-clock seconds as computed by the gateway, when present.
    public var elapsedSeconds: Int?
    /// Ordered state transitions, oldest-first as returned by the gateway.
    public var transitions: [JobTransition]

    public init(
        id: String,
        title: String,
        description: String,
        state: String,
        phase: JobPhase,
        createdAt: Date?,
        startedAt: Date?,
        completedAt: Date?,
        elapsedSeconds: Int?,
        transitions: [JobTransition]
    ) {
        self.id = id
        self.title = title
        self.description = description
        self.state = state
        self.phase = phase
        self.createdAt = createdAt
        self.startedAt = startedAt
        self.completedAt = completedAt
        self.elapsedSeconds = elapsedSeconds
        self.transitions = transitions
    }
}

/// Aggregate counts across all jobs (`GET /api/jobs/summary`), driving the
/// glance's header chips.
public struct JobsSummary: Hashable, Sendable {
    public var total: Int
    public var pending: Int
    public var inProgress: Int
    public var completed: Int
    public var failed: Int
    public var cancelled: Int
    public var interrupted: Int
    public var stuck: Int

    public init(
        total: Int = 0,
        pending: Int = 0,
        inProgress: Int = 0,
        completed: Int = 0,
        failed: Int = 0,
        cancelled: Int = 0,
        interrupted: Int = 0,
        stuck: Int = 0
    ) {
        self.total = total
        self.pending = pending
        self.inProgress = inProgress
        self.completed = completed
        self.failed = failed
        self.cancelled = cancelled
        self.interrupted = interrupted
        self.stuck = stuck
    }

    /// Count of jobs still in flight (pending + running) — the glance highlights
    /// this because it is the only actionable-attention number for a read-only
    /// surface.
    public var active: Int { pending + inProgress }
}
