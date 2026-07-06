import Foundation
import ThinClawCore
import ThinClawTransport

#if canImport(LocalAuthentication)
    import LocalAuthentication
#endif

/// Production ``ApprovalsGateway`` backed by the live ``GatewaySession``.
///
/// A thin forwarding shim: the session already owns the generated REST client
/// (`GET /api/chat/approvals`, `POST /api/chat/approval`) and the session-wide
/// `approval_needed` fan-out. Keeping the store behind the protocol lets the
/// respond flow be unit-tested on macOS without a live gateway.
public struct GatewaySessionApprovalsGateway: ApprovalsGateway {
    private let session: GatewaySession

    public init(session: GatewaySession) {
        self.session = session
    }

    public func pendingApprovals() async throws -> [ApprovalRequest] {
        try await session.pendingApprovals()
    }

    public func approvalEvents() -> AsyncStream<ApprovalRequest> {
        // The stream is created inside the actor; hop in to build it.
        AsyncStream { continuation in
            let task = Task {
                for await request in await session.approvalEvents() {
                    continuation.yield(request)
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    public func respondToApproval(
        _ requestID: String, decision: ApprovalDecision, thread: ThreadID?
    ) async throws {
        try await session.respondToApproval(requestID, decision: decision, thread: thread)
    }
}

/// Production ``BiometricGating`` over `LocalAuthentication` (D-K3): a fresh
/// Face ID / Touch ID evaluation with no passcode fallback, so a high-risk
/// approve genuinely requires biometric presence. Any failure, cancel, or
/// unavailable/unenrolled state resolves to `false` — the store then aborts
/// the approval rather than proceeding.
public struct LocalAuthenticationGate: BiometricGating {
    public init() {}

    public func authenticate(reason: String) async -> Bool {
        #if canImport(LocalAuthentication)
            let context = LAContext()
            context.localizedCancelTitle = "Cancel"
            var error: NSError?
            guard
                context.canEvaluatePolicy(
                    .deviceOwnerAuthenticationWithBiometrics, error: &error)
            else {
                return false
            }
            return await withCheckedContinuation { continuation in
                context.evaluatePolicy(
                    .deviceOwnerAuthenticationWithBiometrics,
                    localizedReason: reason
                ) { success, _ in
                    continuation.resume(returning: success)
                }
            }
        #else
            return false
        #endif
    }
}
