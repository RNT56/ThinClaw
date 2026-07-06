import Testing

@testable import ThinClawCore

@Suite("ApprovalDecision")
struct ApprovalDecisionTests {
    @Test("wire strings match the gateway action contract")
    func wireStrings() {
        #expect(ApprovalDecision.approve.wire == "approve")
        #expect(ApprovalDecision.always.wire == "always")
        #expect(ApprovalDecision.deny.wire == "deny")
    }

    @Test("only deny is a non-approval")
    func isApproval() {
        #expect(ApprovalDecision.approve.isApproval)
        #expect(ApprovalDecision.always.isApproval)
        #expect(!ApprovalDecision.deny.isApproval)
    }

    @Test("Face ID gates only a high-risk approve/always, never deny or low-risk")
    func biometricGatePolicy() {
        // High-risk: approving gates, denying does not.
        #expect(ApprovalDecision.approve.requiresBiometricGate(for: .high))
        #expect(ApprovalDecision.always.requiresBiometricGate(for: .high))
        #expect(!ApprovalDecision.deny.requiresBiometricGate(for: .high))

        // Low-risk: nothing gates.
        #expect(!ApprovalDecision.approve.requiresBiometricGate(for: .low))
        #expect(!ApprovalDecision.always.requiresBiometricGate(for: .low))
        #expect(!ApprovalDecision.deny.requiresBiometricGate(for: .low))
    }

    @Test("an unknown/absent wire risk tier defaults to high (never downgrades)")
    func unknownRiskDefaultsHigh() {
        #expect(RiskTier(wire: nil) == .high)
        #expect(RiskTier(wire: "medium") == .high)
        #expect(RiskTier(wire: "low") == .low)
        #expect(RiskTier(wire: "high") == .high)
    }
}
