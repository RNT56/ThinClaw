import Foundation
import ThinClawSnapshotKit

/// The denormalized live state that ``SnapshotPublisher`` projects into the
/// three App Group snapshots (agent status, pending approvals, jobs).
///
/// This is a *pure value* deliberately decoupled from the gateway REST client
/// and the SSE transport: the app collects it from whatever source is
/// convenient (folded SSE events on the foreground path, one-shot REST fetches
/// on the silent-push / `BGAppRefresh` path) and hands it to the publisher.
/// Keeping the seam here — rather than passing `AgentEvent`s or generated API
/// DTOs into the publisher — is what makes the mapping macOS-testable with
/// scripted fixtures and keeps ThinClawCore free of any networking dependency.
public struct SnapshotInputs: Sendable, Equatable {
    /// A background job as surfaced to the Jobs widget. A small denormalized
    /// projection — never the full job record.
    public struct Job: Sendable, Equatable {
        public var id: String
        public var title: String
        public var phase: JobsSnapshot.JobSummary.Phase
        public var startedAt: Date

        public init(
            id: String,
            title: String,
            phase: JobsSnapshot.JobSummary.Phase,
            startedAt: Date
        ) {
            self.id = id
            self.title = title
            self.phase = phase
            self.startedAt = startedAt
        }
    }

    // MARK: Agent status

    /// What the agent is doing right now.
    public var phase: AgentStatusSnapshot.Phase
    /// Tool currently executing (only meaningful when `phase == .runningTool`).
    public var activeToolName: String?
    /// Thread the agent is currently working, if any.
    public var activeThreadID: ThreadID?
    /// Title of that thread, denormalized for glanceable rendering.
    public var activeThreadTitle: String?
    /// Unseen assistant replies since the app was last foregrounded.
    public var unreadCount: Int

    // MARK: Approvals

    /// Pending tool-call approvals (oldest-first, as the approvals store holds
    /// them).
    public var pendingApprovals: [ApprovalRequest]

    // MARK: Jobs

    /// Active/known background jobs.
    public var jobs: [Job]

    public init(
        phase: AgentStatusSnapshot.Phase = .idle,
        activeToolName: String? = nil,
        activeThreadID: ThreadID? = nil,
        activeThreadTitle: String? = nil,
        unreadCount: Int = 0,
        pendingApprovals: [ApprovalRequest] = [],
        jobs: [Job] = []
    ) {
        self.phase = phase
        self.activeToolName = activeToolName
        self.activeThreadID = activeThreadID
        self.activeThreadTitle = activeThreadTitle
        self.unreadCount = unreadCount
        self.pendingApprovals = pendingApprovals
        self.jobs = jobs
    }
}

/// The preview-privacy setting (docs/MOBILE_SECURITY.md, D-N3 / "Data at rest").
///
/// Widget snapshots persist to the App Group container, so they are subject to
/// the same "content-free by default" discipline as pushes: they may carry a
/// thread *title*, an approval *title + risk badge*, and a **truncated**
/// preview — never a full transcript fragment. This policy centralizes the two
/// knobs the publisher honors:
/// - `previewsEnabled`: when `false`, human-authored text (thread titles,
///   approval descriptions) is dropped entirely — the widget shows only status
///   enums, counts, and risk badges. When `true`, that text is admitted but
///   still truncated.
/// - `previewCharacterLimit`: the hard cap applied to any admitted text.
public struct SnapshotPrivacyPolicy: Sendable, Equatable {
    /// Whether human-authored preview text (titles, descriptions) may appear in
    /// snapshots at all.
    public var previewsEnabled: Bool
    /// Maximum character count for any admitted preview string. Longer strings
    /// are truncated with an ellipsis.
    public var previewCharacterLimit: Int

    public init(previewsEnabled: Bool = true, previewCharacterLimit: Int = 80) {
        self.previewsEnabled = previewsEnabled
        self.previewCharacterLimit = max(1, previewCharacterLimit)
    }

    /// The default: previews on, 80-character cap — enough for a glanceable
    /// title without shipping a paragraph to disk.
    public static let `default` = SnapshotPrivacyPolicy()

    /// "App only" previews (D-N3): no human-authored text leaves the app process
    /// into the shared container. Widgets render status/counts/risk only.
    public static let redacted = SnapshotPrivacyPolicy(previewsEnabled: false)

    /// Apply the policy to a piece of human-authored preview text: `nil` when
    /// previews are disabled or the input is `nil`/blank, otherwise the input
    /// truncated to the character limit (ellipsized when it was cut).
    public func admit(_ text: String?) -> String? {
        guard previewsEnabled, let text else { return nil }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        return Self.truncate(trimmed, to: previewCharacterLimit)
    }

    /// Truncate `text` to at most `limit` characters, appending an ellipsis when
    /// truncation occurred. The ellipsis counts toward the limit so the result
    /// never exceeds `limit`.
    static func truncate(_ text: String, to limit: Int) -> String {
        guard text.count > limit else { return text }
        guard limit > 1 else { return String(text.prefix(limit)) }
        return String(text.prefix(limit - 1)) + "\u{2026}"
    }
}

extension SnapshotInputs {
    /// Project the collected state into the three snapshots, stamping each with
    /// `generatedAt` and applying `privacy` truncation to every human-authored
    /// string. Pure and deterministic — the whole mapping is exercised by
    /// `swift test` on macOS.
    public func project(
        at generatedAt: Date,
        privacy: SnapshotPrivacyPolicy
    ) -> ProjectedSnapshots {
        let status = AgentStatusSnapshot(
            generatedAt: generatedAt,
            phase: phase,
            activeToolName: activeToolName,
            activeThreadID: activeThreadID?.rawValue,
            activeThreadTitle: privacy.admit(activeThreadTitle),
            unreadCount: max(0, unreadCount))

        let approvals = PendingApprovalsSnapshot(
            generatedAt: generatedAt,
            approvals: pendingApprovals.map { request in
                PendingApprovalsSnapshot.PendingApproval(
                    id: request.requestID,
                    // Tool name is a code identifier, not operator prose, so it
                    // is always admitted (the widget needs *something* to label
                    // the row even in "app only" preview mode); only the
                    // free-text description is preview-gated.
                    toolName: request.toolName,
                    description: privacy.admit(request.description) ?? "",
                    threadID: request.threadID?.rawValue,
                    requestedAt: generatedAt,
                    // Denormalize the gateway-computed tier so the widget can
                    // gate inline approve without a re-fetch (D-K3). Core's
                    // RiskTier maps 1:1 to the snapshot's fail-closed tier.
                    risk: request.risk.snapshotRisk)
            })

        let jobs = JobsSnapshot(
            generatedAt: generatedAt,
            jobs: jobs.map { job in
                JobsSnapshot.JobSummary(
                    id: job.id,
                    title: privacy.admit(job.title) ?? "Job",
                    phase: job.phase,
                    startedAt: job.startedAt)
            })

        return ProjectedSnapshots(status: status, approvals: approvals, jobs: jobs)
    }
}

extension RiskTier {
    /// Map the domain risk tier onto the snapshot layer's tier. Both are
    /// fail-closed (`.high` on anything unrecognized), so the mapping is total
    /// and never downgrades an approval.
    var snapshotRisk: PendingApprovalsSnapshot.RiskTier {
        switch self {
        case .low: .low
        case .high: .high
        }
    }
}

/// The three snapshots produced by one projection pass, bundled so the
/// publisher writes them as a coherent set stamped with a single `generatedAt`.
public struct ProjectedSnapshots: Sendable, Equatable {
    public var status: AgentStatusSnapshot
    public var approvals: PendingApprovalsSnapshot
    public var jobs: JobsSnapshot

    public init(
        status: AgentStatusSnapshot,
        approvals: PendingApprovalsSnapshot,
        jobs: JobsSnapshot
    ) {
        self.status = status
        self.approvals = approvals
        self.jobs = jobs
    }
}
