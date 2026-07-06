import Foundation
import ThinClawSnapshotKit

/// Packs the two watch-glanceable snapshots — agent status and pending
/// approvals — into a single `WCSession` application context, and decodes them
/// back on the watch. Pure Foundation + SnapshotKit, so the pack/unpack is
/// macOS-testable without WatchConnectivity.
///
/// The snapshots are already content-minimised by the app's `SnapshotPrivacyPolicy`
/// before they reach here (D-N / data-at-rest); this helper only transports them.
/// Only status + low-risk-approvable entries are meaningful on the watch, but the
/// risk tier travels so the watch can gate exactly like the phone (D-K3): a
/// missing/high tier is never offered an inline approve.
public enum WatchSnapshotMirror {
    static let statusKey = "watchAgentStatus"
    static let approvalsKey = "watchPendingApprovals"

    /// Build an application-context dictionary carrying both snapshots.
    public static func applicationContext(
        status: AgentStatusSnapshot,
        approvals: PendingApprovalsSnapshot
    ) throws -> [String: Any] {
        let encoder = JSONEncoder()
        return [
            statusKey: try encoder.encode(status),
            approvalsKey: try encoder.encode(approvals),
        ]
    }

    /// Decode the mirrored status snapshot from a received context, or `nil` if
    /// this context carries none (it may be a provisioning-only context).
    public static func status(from context: [String: Any]) -> AgentStatusSnapshot? {
        guard let data = context[statusKey] as? Data else { return nil }
        return try? JSONDecoder().decode(AgentStatusSnapshot.self, from: data)
    }

    /// Decode the mirrored approvals snapshot from a received context, or `nil`.
    public static func approvals(from context: [String: Any]) -> PendingApprovalsSnapshot? {
        guard let data = context[approvalsKey] as? Data else { return nil }
        return try? JSONDecoder().decode(PendingApprovalsSnapshot.self, from: data)
    }
}
