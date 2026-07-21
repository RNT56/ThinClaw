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
    public private(set) var submittingRequestIDs: Set<String> = []
    public private(set) var notice: String?
    public private(set) var errorMessage: String?

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
        // Let the subscription task attach its stream continuation before an
        // immediate producer can emit. This closes the cold-start race where a
        // live approval arriving between launch and the first snapshot fetch
        // could otherwise be lost.
        await Task.yield()
        await refresh()
    }

    /// Stop folding live events.
    public func stop() {
        eventTask?.cancel()
        eventTask = nil
    }

    /// Re-pull the authoritative pending set. Requests with a local decision in
    /// flight are retained until that decision completes; everything else
    /// absent from the server snapshot is resolved or expired and is removed.
    public func refresh() async {
        let fetched: [ApprovalRequest]
        do {
            fetched = try await gateway.pendingApprovals()
        } catch {
            errorMessage = "Couldn’t refresh approvals. Check the connection and try again."
            return
        }
        errorMessage = nil
        let previousIDs = Set(pending.map(\.requestID))
        let fetchedIDs = Set(fetched.map(\.requestID))
        let inFlightOnly = pending.filter {
            submittingRequestIDs.contains($0.requestID) && !fetchedIDs.contains($0.requestID)
        }
        pending = fetched + inFlightOnly
        let resolvedElsewhere =
            previousIDs
            .subtracting(fetchedIDs)
            .subtracting(submittingRequestIDs)
        if !resolvedElsewhere.isEmpty {
            notice = "The approval list changed because a request was resolved elsewhere or expired."
        }
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
            notice = "That request is no longer pending."
            return false
        }

        if decision.requiresBiometricGate(for: request.risk) {
            let ok = await biometrics.authenticate(
                reason: "Approve \(request.toolName)")
            guard ok else {
                notice = "Authentication wasn’t completed. The request is still pending."
                return false
            }
        }

        errorMessage = nil
        submittingRequestIDs.insert(requestID)
        do {
            try await gateway.respondToApproval(
                requestID, decision: decision, thread: request.threadID)
        } catch {
            submittingRequestIDs.remove(requestID)
            // Another surface may have resolved it first. Refresh before
            // declaring failure; absence from the authoritative list means the
            // desired terminal state has already been reached.
            await refresh()
            if !pending.contains(where: { $0.requestID == requestID }) {
                notice = "This request was already resolved elsewhere."
                return true
            }
            errorMessage = "Couldn’t submit the decision. The request is still pending."
            return false
        }

        submittingRequestIDs.remove(requestID)
        remove(requestID)
        notice = decision == .deny ? "Request denied." : "Request approved."
        return true
    }

    public func clearMessages() {
        notice = nil
        errorMessage = nil
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
}
