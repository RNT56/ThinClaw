import Foundation
import Testing

@testable import ThinClawCore

// MARK: - Test doubles

/// Records `respondToApproval` calls and serves a scripted pending set + a
/// controllable live event stream, so the store's respond flow is exercised
/// without a live gateway.
private final class MockApprovalsGateway: ApprovalsGateway, @unchecked Sendable {
    struct Call: Equatable {
        let requestID: String
        let decision: ApprovalDecision
        let thread: ThreadID?
    }

    private let lock = NSLock()
    private var _pending: [ApprovalRequest]
    private var _calls: [Call] = []
    private var _shouldThrowOnRespond = false
    private var continuation: AsyncStream<ApprovalRequest>.Continuation?

    init(pending: [ApprovalRequest] = []) {
        self._pending = pending
    }

    var calls: [Call] {
        lock.withLock { _calls }
    }

    func setPending(_ pending: [ApprovalRequest]) {
        lock.withLock { _pending = pending }
    }

    func failNextRespond() {
        lock.withLock { _shouldThrowOnRespond = true }
    }

    /// Push a live `approval_needed` into the store's subscription.
    func emit(_ request: ApprovalRequest) {
        continuation?.yield(request)
    }

    func pendingApprovals() async throws -> [ApprovalRequest] {
        lock.withLock { _pending }
    }

    func approvalEvents() -> AsyncStream<ApprovalRequest> {
        AsyncStream { continuation in
            self.continuation = continuation
        }
    }

    func respondToApproval(
        _ requestID: String, decision: ApprovalDecision, thread: ThreadID?
    ) async throws {
        let shouldThrow = lock.withLock { _shouldThrowOnRespond }
        if shouldThrow {
            lock.withLock { _shouldThrowOnRespond = false }
            struct RespondFailed: Error {}
            throw RespondFailed()
        }
        lock.withLock {
            _calls.append(Call(requestID: requestID, decision: decision, thread: thread))
        }
    }
}

/// A biometric gate whose result and invocation count are controlled by the
/// test — no device needed.
private final class MockBiometricGate: BiometricGating, @unchecked Sendable {
    private let lock = NSLock()
    private var _result: Bool
    private var _invocations = 0

    init(result: Bool) {
        self._result = result
    }

    var invocations: Int { lock.withLock { _invocations } }

    func setResult(_ value: Bool) { lock.withLock { _result = value } }

    func authenticate(reason: String) async -> Bool {
        lock.withLock {
            _invocations += 1
            return _result
        }
    }
}

private func request(
    _ id: String,
    tool: String = "shell",
    risk: RiskTier = .low,
    thread: ThreadID? = ThreadID("th_1")
) -> ApprovalRequest {
    ApprovalRequest(
        requestID: id, toolName: tool, description: "desc",
        parameters: "{}", risk: risk, threadID: thread)
}

// MARK: - Tests

@MainActor
@Suite("ApprovalsStore")
struct ApprovalsStoreTests {
    @Test("cold-load populates the pending set from the gateway")
    func coldLoad() async {
        let gateway = MockApprovalsGateway(pending: [request("r1"), request("r2")])
        let store = ApprovalsStore(gateway: gateway, biometrics: MockBiometricGate(result: true))

        await store.refresh()

        #expect(store.pending.map(\.requestID) == ["r1", "r2"])
        #expect(store.badgeCount == 2)
    }

    @Test("refresh replaces the authoritative snapshot and removes resolved entries")
    func authoritativeReplacement() async {
        let gateway = MockApprovalsGateway(pending: [request("r1"), request("r2")])
        let store = ApprovalsStore(gateway: gateway, biometrics: MockBiometricGate(result: true))
        await store.refresh()
        gateway.setPending([request("r2")])

        await store.refresh()

        #expect(store.pending.map(\.requestID) == ["r2"])
    }

    @Test("low-risk approve submits without a biometric prompt and removes the entry")
    func lowRiskApproveNoGate() async {
        let gateway = MockApprovalsGateway(pending: [request("r1", risk: .low)])
        let gate = MockBiometricGate(result: true)
        let store = ApprovalsStore(gateway: gateway, biometrics: gate)
        await store.refresh()

        let ok = await store.approve("r1")

        #expect(ok)
        #expect(gate.invocations == 0, "low-risk approve must not gate")
        #expect(gateway.calls == [.init(requestID: "r1", decision: .approve, thread: ThreadID("th_1"))])
        #expect(store.pending.isEmpty)
    }

    @Test("high-risk approve requires biometric success before the POST")
    func highRiskApproveGatesAndSucceeds() async {
        let gateway = MockApprovalsGateway(pending: [request("r1", risk: .high)])
        let gate = MockBiometricGate(result: true)
        let store = ApprovalsStore(gateway: gateway, biometrics: gate)
        await store.refresh()

        let ok = await store.approve("r1")

        #expect(ok)
        #expect(gate.invocations == 1, "high-risk approve must gate exactly once")
        #expect(gateway.calls.count == 1)
        #expect(store.pending.isEmpty)
    }

    @Test("a failed biometric aborts a high-risk approve without hitting the gateway")
    func highRiskApproveBiometricDenied() async {
        let gateway = MockApprovalsGateway(pending: [request("r1", risk: .high)])
        let gate = MockBiometricGate(result: false)
        let store = ApprovalsStore(gateway: gateway, biometrics: gate)
        await store.refresh()

        let ok = await store.approve("r1")

        #expect(!ok)
        #expect(gate.invocations == 1)
        #expect(gateway.calls.isEmpty, "no POST when biometrics fail")
        #expect(store.pending.map(\.requestID) == ["r1"], "entry stays for retry")
    }

    @Test("deny never gates, even for a high-risk request")
    func highRiskDenyNoGate() async {
        let gateway = MockApprovalsGateway(pending: [request("r1", risk: .high)])
        let gate = MockBiometricGate(result: false)
        let store = ApprovalsStore(gateway: gateway, biometrics: gate)
        await store.refresh()

        let ok = await store.deny("r1")

        #expect(ok)
        #expect(gate.invocations == 0, "deny must never gate")
        #expect(gateway.calls == [.init(requestID: "r1", decision: .deny, thread: ThreadID("th_1"))])
        #expect(store.pending.isEmpty)
    }

    @Test("always is treated as a gated approval for high risk")
    func alwaysGatesHighRisk() async {
        let gateway = MockApprovalsGateway(pending: [request("r1", risk: .high)])
        let gate = MockBiometricGate(result: true)
        let store = ApprovalsStore(gateway: gateway, biometrics: gate)
        await store.refresh()

        let ok = await store.respond("r1", decision: .always)

        #expect(ok)
        #expect(gate.invocations == 1)
        #expect(gateway.calls.first?.decision == .always)
    }

    @Test("a failed POST keeps the entry so the operator can retry")
    func failedPostKeepsEntry() async {
        let gateway = MockApprovalsGateway(pending: [request("r1", risk: .low)])
        let store = ApprovalsStore(gateway: gateway, biometrics: MockBiometricGate(result: true))
        await store.refresh()
        gateway.failNextRespond()

        let ok = await store.approve("r1")

        #expect(!ok)
        #expect(store.pending.map(\.requestID) == ["r1"])
    }

    @Test("responding to an unknown request id is a no-op")
    func unknownRequestID() async {
        let gateway = MockApprovalsGateway()
        let gate = MockBiometricGate(result: true)
        let store = ApprovalsStore(gateway: gateway, biometrics: gate)

        let ok = await store.approve("nope")

        #expect(!ok)
        #expect(gate.invocations == 0)
        #expect(gateway.calls.isEmpty)
    }

    @Test("a live approval_needed event upserts into the pending set")
    func liveEventUpserts() async {
        let gateway = MockApprovalsGateway()
        let store = ApprovalsStore(gateway: gateway, biometrics: MockBiometricGate(result: true))
        await store.start()

        gateway.emit(request("live1", risk: .high))
        // Yield so the subscription task drains the emitted event.
        await Task.yield()
        await Task.yield()

        #expect(store.pending.map(\.requestID) == ["live1"])
        store.stop()
    }
}
