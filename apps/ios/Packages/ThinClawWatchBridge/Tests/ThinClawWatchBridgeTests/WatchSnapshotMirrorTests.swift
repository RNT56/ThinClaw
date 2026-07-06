import Foundation
import Testing
import ThinClawSnapshotKit

@testable import ThinClawWatchBridge

@Suite("WatchSnapshotMirror pack/unpack")
struct WatchSnapshotMirrorTests {
    private func status() -> AgentStatusSnapshot {
        AgentStatusSnapshot(
            generatedAt: Date(timeIntervalSince1970: 100),
            phase: .waitingForApproval,
            activeThreadID: "t-1",
            unreadCount: 2)
    }

    private func approvals() -> PendingApprovalsSnapshot {
        PendingApprovalsSnapshot(
            generatedAt: Date(timeIntervalSince1970: 100),
            approvals: [
                .init(
                    id: "a-1", toolName: "read_file", description: "",
                    requestedAt: Date(timeIntervalSince1970: 90), risk: .low)
            ])
    }

    @Test("Both snapshots round-trip through an application context")
    func roundTrips() throws {
        let context = try WatchSnapshotMirror.applicationContext(
            status: status(), approvals: approvals())

        let decodedStatus = WatchSnapshotMirror.status(from: context)
        let decodedApprovals = WatchSnapshotMirror.approvals(from: context)

        #expect(decodedStatus?.phase == .waitingForApproval)
        #expect(decodedStatus?.unreadCount == 2)
        #expect(decodedApprovals?.approvals.first?.id == "a-1")
        #expect(decodedApprovals?.approvals.first?.effectiveRisk == .low)
    }

    @Test("A provisioning-only context yields nil snapshots")
    func absentSnapshotsNil() {
        let context: [String: Any] = ["companionProvisioning": Data()]
        #expect(WatchSnapshotMirror.status(from: context) == nil)
        #expect(WatchSnapshotMirror.approvals(from: context) == nil)
    }

    @Test("A missing risk tier decodes fail-closed to high (no off-device approve)")
    func missingRiskIsHigh() throws {
        // A snapshot written before the risk field existed: the watch must read
        // it back as high so it never offers an inline approve (D-K3/D-K4).
        let legacy = PendingApprovalsSnapshot(
            generatedAt: Date(timeIntervalSince1970: 100),
            approvals: [
                .init(
                    id: "a-2", toolName: "deploy", description: "",
                    requestedAt: Date(timeIntervalSince1970: 90), risk: nil)
            ])
        let context = try WatchSnapshotMirror.applicationContext(
            status: status(), approvals: legacy)
        let back = WatchSnapshotMirror.approvals(from: context)
        #expect(back?.approvals.first?.effectiveRisk == .high)
    }
}
