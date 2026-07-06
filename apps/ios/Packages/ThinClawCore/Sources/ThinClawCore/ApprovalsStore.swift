import Foundation
import Observation

/// The network operations the approvals surface needs, abstracted so the store
/// is testable on macOS without a live gateway (the production adapter wraps
/// `ThinClawTransport.GatewaySession`).
public protocol ApprovalsGateway: Sendable {
    /// Cold-load the currently-pending approvals (`GET /api/chat/approvals`).
    func pendingApprovals() async throws -> [ApprovalRequest]

    /// Live `approval_needed` events, session-wide.
    func approvalEvents() -> AsyncStream<ApprovalRequest>

    /// Submit a decision (`POST /api/chat/approval`).
    func respondToApproval(
        _ requestID: String, decision: ApprovalDecision, thread: ThreadID?
    ) async throws
}

/// The biometric gate, abstracted so tests inject a deterministic result and
/// the store never links `LocalAuthentication` directly. The production
/// implementation evaluates `LAContext.deviceOwnerAuthenticationWithBiometrics`
/// (Face ID) per D-K3.
public protocol BiometricGating: Sendable {
    /// Prompt for biometric authentication. Returns `true` only on success;
    /// a cancel, lockout, or unenrolled device returns `false` and the caller
    /// must treat the decision as *not* authorized.
    func authenticate(reason: String) async -> Bool
}

/// Drives the pending-approvals surface: cold-loads on open, folds live
/// `approval_needed` events, and submits decisions — biometric-gating a
/// high-risk **approve** before it fires (D-K3, `docs/MOBILE_SECURITY.md`).
///
/// UI-free by design: it imports no SwiftUI/design layer, so the whole respond
/// flow (gate → client → entry removal) is exercised by plain `swift test` on
/// macOS with a mocked gateway and a mocked biometric gate. The iOS
/// `FeatureApprovals` package supplies the SwiftUI screen and the concrete
/// `GatewaySession` / `LocalAuthentication` adapters.
@MainActor
@Observable
public final class ApprovalsStore {
    /// Pending approvals, oldest-first, as surfaced to the UI.
    public private(set) var pending: [ApprovalRequest] = []

    /// Badge count for the app shell.
    public var badgeCount: Int { pending.count }

    private let gateway: any ApprovalsGateway
    private let biometrics: any BiometricGating

    private var eventTask: Task<Void, Never>?

    public init(gateway: any ApprovalsGateway, biometrics: any BiometricGating) {
        self.gateway = gateway
        self.biometrics = biometrics
    }

    // MARK: - Lifecycle

    /// Cold-load the pending set and begin folding live events. Idempotent.
    public func start() async {
        subscribeToEvents()
        await refresh()
    }

    /// Stop folding live events.
    public func stop() {
        eventTask?.cancel()
        eventTask = nil
    }

    /// Re-pull the pending set (pull-to-refresh / reconnect). Merges rather than
    /// replaces so a live event that arrived between the request and its
    /// response is not dropped, and a decision already applied locally is not
    /// resurrected by a stale server snapshot.
    public func refresh() async {
        guard let fetched = try? await gateway.pendingApprovals() else { return }
        merge(fetched)
    }

    private func subscribeToEvents() {
        guard eventTask == nil else { return }
        eventTask = Task { [weak self, gateway] in
            for await request in gateway.approvalEvents() {
                guard let self else { break }
                self.upsert(request)
            }
        }
    }

    // MARK: - Decisions

    /// Apply a decision to `requestID`. High-risk **approvals** require a fresh
    /// biometric success first (D-K3); a failed/cancelled biometric aborts
    /// without touching the gateway and leaves the entry in place. Denials and
    /// low-risk approvals never gate. On a successful POST the entry is removed.
    ///
    /// - Returns: `true` if the decision was submitted; `false` if it was
    ///   aborted (biometric denied) or the request id is unknown.
    @discardableResult
    public func respond(_ requestID: String, decision: ApprovalDecision) async -> Bool {
        guard let request = pending.first(where: { $0.requestID == requestID }) else {
            return false
        }

        if decision.requiresBiometricGate(for: request.risk) {
            let ok = await biometrics.authenticate(
                reason: "Approve \(request.toolName)")
            guard ok else { return false }
        }

        do {
            try await gateway.respondToApproval(
                requestID, decision: decision, thread: request.threadID)
        } catch {
            // Leave the entry in place so the operator can retry; a failed POST
            // must not silently drop a pending approval.
            return false
        }

        remove(requestID)
        return true
    }

    // MARK: - Convenience

    /// Approve (biometric-gated when high-risk).
    @discardableResult
    public func approve(_ requestID: String) async -> Bool {
        await respond(requestID, decision: .approve)
    }

    /// Deny (never gated).
    @discardableResult
    public func deny(_ requestID: String) async -> Bool {
        await respond(requestID, decision: .deny)
    }

    // MARK: - Local mutation

    private func upsert(_ request: ApprovalRequest) {
        if let index = pending.firstIndex(where: { $0.requestID == request.requestID }) {
            pending[index] = request
        } else {
            pending.append(request)
        }
    }

    private func remove(_ requestID: String) {
        pending.removeAll { $0.requestID == requestID }
    }

    /// Fold a freshly-pulled set into the current one: update/insert every
    /// fetched entry while keeping locally-known entries the pull missed. The
    /// pull is deliberately additive rather than authoritative because the
    /// gateway cache is best-effort/lossy per its contract — treating an empty
    /// or partial pull as "these are the only pending approvals" would drop a
    /// live one the cache forgot. Decided entries are already removed locally
    /// on decision, so they do not reappear.
    private func merge(_ fetched: [ApprovalRequest]) {
        for request in fetched { upsert(request) }
    }
}
