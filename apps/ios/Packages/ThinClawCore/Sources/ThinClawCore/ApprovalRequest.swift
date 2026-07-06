import Foundation

/// Gateway-computed risk tier for a tool approval (docs/MOBILE_SECURITY.md,
/// D-K3). This is the single source of truth on the client: it drives the
/// biometric gate and the push category, and every surface (chat card, widget,
/// watch) reads it from here.
///
/// Serialized snake_case (`"low"` / `"high"`) on `approval_needed` events and
/// on `PendingApprovalEntry`. The tier is *always* present on the wire, but an
/// unknown or missing value decodes to ``high`` — the safe default: an
/// unrecognized tier must never silently downgrade a high-risk approval.
public enum RiskTier: String, Sendable, Hashable, Codable, CaseIterable {
    case low
    case high

    /// Map a wire string to a tier, defaulting unknown/absent values to
    /// ``high`` so a new or garbled gateway value never bypasses the gate.
    public init(wire: String?) {
        switch wire {
        case "low": self = .low
        case "high": self = .high
        default: self = .high
        }
    }
}

/// A pending tool-call approval surfaced by the gateway (`approval_needed`).
///
/// `parameters` is a JSON-encoded string exactly as the gateway sends it
/// (the gateway serializes tool parameters to a string before embedding them
/// in the event); the UI pretty-prints it but the client never needs to
/// interpret it structurally.
public struct ApprovalRequest: Hashable, Sendable, Codable, Identifiable {
    /// Gateway-issued request id; echo it back on `/api/chat/approval`.
    public var requestID: String
    public var toolName: String
    public var description: String
    /// JSON-encoded tool parameters, verbatim from the gateway.
    public var parameters: String
    /// Gateway-computed risk tier; high-risk approvals require a biometric
    /// gate before the decision fires.
    public var risk: RiskTier
    public var threadID: ThreadID?

    public var id: String { requestID }

    public init(
        requestID: String,
        toolName: String,
        description: String,
        parameters: String,
        risk: RiskTier,
        threadID: ThreadID? = nil
    ) {
        self.requestID = requestID
        self.toolName = toolName
        self.description = description
        self.parameters = parameters
        self.risk = risk
        self.threadID = threadID
    }
}

/// The operator's decision on a pending approval, submitted to
/// `POST /api/chat/approval` as the `action` field.
///
/// `always` is "approve and remember" (persist an allow rule for this tool);
/// the gateway treats it as an approve for the current call plus a policy
/// write. The raw wire string matches the gateway's `ApprovalRequest.action`
/// contract (`"approve"`, `"always"`, `"deny"`).
public enum ApprovalDecision: String, Sendable, Hashable, Codable, CaseIterable {
    case approve
    case always
    case deny

    /// The `action` string echoed back to the gateway.
    public var wire: String { rawValue }

    /// Whether this decision is an approval (grants the tool call). `deny`
    /// never grants; `approve`/`always` do.
    public var isApproval: Bool { self != .deny }

    /// Whether this decision requires a fresh biometric gate before it fires,
    /// given the request's risk tier (D-K3, `docs/MOBILE_SECURITY.md`).
    ///
    /// Face ID is required *only* to **approve** a **high-risk** tool call.
    /// Denials never gate (blocking a tool must always be frictionless), and
    /// low-risk approvals never gate. This is the single policy predicate the
    /// approvals store consults before calling the biometric gate.
    public func requiresBiometricGate(for risk: RiskTier) -> Bool {
        isApproval && risk == .high
    }
}
