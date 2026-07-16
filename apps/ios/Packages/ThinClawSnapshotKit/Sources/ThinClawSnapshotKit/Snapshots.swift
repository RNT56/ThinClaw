import Foundation

// MARK: - Agent status

/// What the agent is doing right now — drives the AgentStatus widget, the
/// watch complication, and the Live Activity's fallback state.
public struct AgentStatusSnapshot: SharedSnapshot, GatewayScopedSnapshot, FreshnessAwareSnapshot {
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
    public var gatewayInstanceID: String?
    public var stale: Bool?
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
        gatewayInstanceID: String? = nil,
        stale: Bool = false,
        generatedAt: Date,
        phase: Phase,
        activeToolName: String? = nil,
        activeThreadID: String? = nil,
        activeThreadTitle: String? = nil,
        unreadCount: Int = 0
    ) {
        self.schemaVersion = schemaVersion
        self.gatewayInstanceID = gatewayInstanceID
        self.stale = stale
        self.generatedAt = generatedAt
        self.phase = phase
        self.activeToolName = activeToolName
        self.activeThreadID = activeThreadID
        self.activeThreadTitle = activeThreadTitle
        self.unreadCount = unreadCount
    }

    public var isKnownStale: Bool { stale ?? false }
}

// MARK: - Pending approvals

/// Tool calls blocked on operator approval — drives the PendingApprovals
/// widget and the watch approval list.
public struct PendingApprovalsSnapshot: SharedSnapshot, GatewayScopedSnapshot, FreshnessAwareSnapshot {
    public static let fileName = "pending-approvals.json"
    public static let currentSchemaVersion = 1

    /// Gateway-computed approval risk tier (D-K3), denormalized into the
    /// snapshot so widget/watch surfaces can enforce "no high-risk approval
    /// off-device" locally. The tier is the gateway's single source of truth
    /// (`thinclaw_gateway::web::devices::approval_risk::classify`); this enum
    /// mirrors the API `ApprovalRisk` without pulling the generated API
    /// package into the snapshot layer.
    ///
    /// Decodes **fail-closed**: an absent or unrecognized value reads back as
    /// ``high`` so a reader can never be tricked into offering an inline
    /// approve for an entry whose tier it does not understand.
    public enum RiskTier: String, Codable, Sendable, Equatable {
        case low
        case high

        public init(from decoder: any Decoder) throws {
            let raw = try decoder.singleValueContainer().decode(String.self)
            self = RiskTier(rawValue: raw) ?? .high
        }
    }

    public struct PendingApproval: Codable, Sendable, Equatable, Identifiable {
        /// Gateway approval request id (echoed to `/api/chat/approval`).
        public var id: String
        public var toolName: String
        public var description: String
        public var threadID: String?
        public var requestedAt: Date
        /// Gateway-computed risk tier (D-K3). Optional for forward/backward
        /// compatibility with snapshots written before the field existed;
        /// readers MUST treat a missing tier as high-risk (see
        /// ``effectiveRisk``), never as approvable off-device.
        public var risk: RiskTier?

        public init(
            id: String,
            toolName: String,
            description: String,
            threadID: String? = nil,
            requestedAt: Date,
            risk: RiskTier? = nil
        ) {
            self.id = id
            self.toolName = toolName
            self.description = description
            self.threadID = threadID
            self.requestedAt = requestedAt
            self.risk = risk
        }

        /// The risk tier a reader must enforce: the stamped tier, or ``high``
        /// when absent. Interactive approve is offered only when this is
        /// ``low`` (docs/MOBILE_SECURITY.md D-K3).
        public var effectiveRisk: RiskTier { risk ?? .high }
    }

    public var schemaVersion: Int
    public var gatewayInstanceID: String?
    public var stale: Bool?
    public var generatedAt: Date
    public var approvals: [PendingApproval]

    public init(
        schemaVersion: Int = Self.currentSchemaVersion,
        gatewayInstanceID: String? = nil,
        stale: Bool = false,
        generatedAt: Date,
        approvals: [PendingApproval]
    ) {
        self.schemaVersion = schemaVersion
        self.gatewayInstanceID = gatewayInstanceID
        self.stale = stale
        self.generatedAt = generatedAt
        self.approvals = approvals
    }

    public var isKnownStale: Bool { stale ?? false }
}

// MARK: - Jobs

/// Long-running background jobs (sandbox jobs, subagents) — drives the Jobs
/// tab's widget surface.
public struct JobsSnapshot: SharedSnapshot, GatewayScopedSnapshot, FreshnessAwareSnapshot {
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
    public var gatewayInstanceID: String?
    public var stale: Bool?
    public var generatedAt: Date
    public var jobs: [JobSummary]

    public init(
        schemaVersion: Int = Self.currentSchemaVersion,
        gatewayInstanceID: String? = nil,
        stale: Bool = false,
        generatedAt: Date,
        jobs: [JobSummary]
    ) {
        self.schemaVersion = schemaVersion
        self.gatewayInstanceID = gatewayInstanceID
        self.stale = stale
        self.generatedAt = generatedAt
        self.jobs = jobs
    }

    public var isKnownStale: Bool { stale ?? false }
}

// MARK: - Quick Ask receipt

/// Written by the QuickAsk widget/intent after handing a prompt to the app
/// (or queuing it for next launch), so the widget can render "sent ✓" state.
public struct QuickAskReceipt: SharedSnapshot, GatewayScopedSnapshot {
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
    public var gatewayInstanceID: String?
    public var generatedAt: Date
    public var text: String
    public var threadID: String?
    public var deliveryState: DeliveryState

    public init(
        schemaVersion: Int = Self.currentSchemaVersion,
        gatewayInstanceID: String? = nil,
        generatedAt: Date,
        text: String,
        threadID: String? = nil,
        deliveryState: DeliveryState
    ) {
        self.schemaVersion = schemaVersion
        self.gatewayInstanceID = gatewayInstanceID
        self.generatedAt = generatedAt
        self.text = text
        self.threadID = threadID
        self.deliveryState = deliveryState
    }
}

/// Encrypted prompt envelopes shared between the widget and main app. The
/// ciphertext is safe to place in the App Group; its AES-GCM key lives only in
/// the shared device-only Keychain.
public struct EncryptedQuickAskQueue: SharedSnapshot {
    public static let fileName = "quick-ask-outbox.json"
    public static let currentSchemaVersion = 1

    public struct Entry: Codable, Sendable, Equatable, Identifiable {
        public var id: UUID
        public var gatewayInstanceID: String
        public var queuedAt: Date
        public var sealedPayload: Data

        public init(
            id: UUID,
            gatewayInstanceID: String,
            queuedAt: Date,
            sealedPayload: Data
        ) {
            self.id = id
            self.gatewayInstanceID = gatewayInstanceID
            self.queuedAt = queuedAt
            self.sealedPayload = sealedPayload
        }
    }

    public var schemaVersion: Int
    public var generatedAt: Date
    public var entries: [Entry]

    public init(
        schemaVersion: Int = Self.currentSchemaVersion,
        generatedAt: Date,
        entries: [Entry]
    ) {
        self.schemaVersion = schemaVersion
        self.generatedAt = generatedAt
        self.entries = entries
    }
}
