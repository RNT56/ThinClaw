import Foundation

// MARK: - Agent status

/// What the agent is doing right now — drives the AgentStatus widget, the
/// watch complication, and the Live Activity's fallback state.
public struct AgentStatusSnapshot: SharedSnapshot {
    public static let fileName = "agent-status.json"
    public static let currentSchemaVersion = 1

    public enum Phase: String, Codable, Sendable {
        case idle
        case thinking
        case streaming
        case runningTool
        case waitingForApproval
        case error
    }

    public var schemaVersion: Int
    public var generatedAt: Date
    public var phase: Phase
    /// Tool currently executing, when `phase == .runningTool`.
    public var activeToolName: String?
    /// Thread the agent is currently working, if any.
    public var activeThreadID: String?
    /// Title of that thread, denormalized for glanceable rendering.
    public var activeThreadTitle: String?
    /// Unseen assistant replies since the app was last foregrounded.
    public var unreadCount: Int

    public init(
        schemaVersion: Int = Self.currentSchemaVersion,
        generatedAt: Date,
        phase: Phase,
        activeToolName: String? = nil,
        activeThreadID: String? = nil,
        activeThreadTitle: String? = nil,
        unreadCount: Int = 0
    ) {
        self.schemaVersion = schemaVersion
        self.generatedAt = generatedAt
        self.phase = phase
        self.activeToolName = activeToolName
        self.activeThreadID = activeThreadID
        self.activeThreadTitle = activeThreadTitle
        self.unreadCount = unreadCount
    }
}

// MARK: - Pending approvals

/// Tool calls blocked on operator approval — drives the PendingApprovals
/// widget and the watch approval list.
public struct PendingApprovalsSnapshot: SharedSnapshot {
    public static let fileName = "pending-approvals.json"
    public static let currentSchemaVersion = 1

    public struct PendingApproval: Codable, Sendable, Equatable, Identifiable {
        /// Gateway approval request id (echoed to `/api/chat/approval`).
        public var id: String
        public var toolName: String
        public var description: String
        public var threadID: String?
        public var requestedAt: Date

        public init(
            id: String,
            toolName: String,
            description: String,
            threadID: String? = nil,
            requestedAt: Date
        ) {
            self.id = id
            self.toolName = toolName
            self.description = description
            self.threadID = threadID
            self.requestedAt = requestedAt
        }
    }

    public var schemaVersion: Int
    public var generatedAt: Date
    public var approvals: [PendingApproval]

    public init(
        schemaVersion: Int = Self.currentSchemaVersion,
        generatedAt: Date,
        approvals: [PendingApproval]
    ) {
        self.schemaVersion = schemaVersion
        self.generatedAt = generatedAt
        self.approvals = approvals
    }
}

// MARK: - Jobs

/// Long-running background jobs (sandbox jobs, subagents) — drives the Jobs
/// tab's widget surface.
public struct JobsSnapshot: SharedSnapshot {
    public static let fileName = "jobs.json"
    public static let currentSchemaVersion = 1

    public struct JobSummary: Codable, Sendable, Equatable, Identifiable {
        public enum Phase: String, Codable, Sendable {
            case queued
            case running
            case succeeded
            case failed
        }

        public var id: String
        public var title: String
        public var phase: Phase
        public var startedAt: Date

        public init(id: String, title: String, phase: Phase, startedAt: Date) {
            self.id = id
            self.title = title
            self.phase = phase
            self.startedAt = startedAt
        }
    }

    public var schemaVersion: Int
    public var generatedAt: Date
    public var jobs: [JobSummary]

    public init(
        schemaVersion: Int = Self.currentSchemaVersion,
        generatedAt: Date,
        jobs: [JobSummary]
    ) {
        self.schemaVersion = schemaVersion
        self.generatedAt = generatedAt
        self.jobs = jobs
    }
}

// MARK: - Quick Ask receipt

/// Written by the QuickAsk widget/intent after handing a prompt to the app
/// (or queuing it for next launch), so the widget can render "sent ✓" state.
public struct QuickAskReceipt: SharedSnapshot {
    public static let fileName = "quick-ask-receipt.json"
    public static let currentSchemaVersion = 1

    public enum DeliveryState: String, Codable, Sendable {
        /// Queued locally; the app will send it on next foreground/refresh.
        case queued
        /// Accepted by the gateway.
        case sent
        /// Send failed; the app surfaces the error.
        case failed
    }

    public var schemaVersion: Int
    public var generatedAt: Date
    public var text: String
    public var threadID: String?
    public var deliveryState: DeliveryState

    public init(
        schemaVersion: Int = Self.currentSchemaVersion,
        generatedAt: Date,
        text: String,
        threadID: String? = nil,
        deliveryState: DeliveryState
    ) {
        self.schemaVersion = schemaVersion
        self.generatedAt = generatedAt
        self.text = text
        self.threadID = threadID
        self.deliveryState = deliveryState
    }
}
