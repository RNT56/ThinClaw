import Foundation
import UserNotifications

/// UNUserNotificationCenter delegate: maps content-free pushes (category +
/// ids only — docs/MOBILE_SECURITY.md D-N1) to deep links and handles the
/// actionable approval categories without foregrounding when possible.
/// Registration and category wiring land at M2.
@MainActor
final class PushCoordinator: NSObject, UNUserNotificationCenterDelegate {
    static let approvalLowCategory = "THINCLAW_APPROVAL"
    static let messageCategory = "THINCLAW_MESSAGE"
    static let jobCategory = "THINCLAW_JOB"

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        // M2: route response.notification userInfo (kind, thread_id,
        // request_id) through AppRouter; handle approve/deny actions
        // in-place via the approvals store.
    }
}
