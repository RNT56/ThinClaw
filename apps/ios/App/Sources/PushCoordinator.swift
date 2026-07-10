import Foundation
import ThinClawAPI
import ThinClawWidgetKitShared
@preconcurrency import UserNotifications

/// `UNUserNotificationCenter` delegate: registers the notification categories,
/// maps content-free pushes (category + ids only — docs/MOBILE_SECURITY.md
/// **D-N1**) to `thinclaw://` deep links, and actions **low-risk** approvals
/// inline without foregrounding.
///
/// The custom payload carries only ids under the `tc` dict (built gateway-side
/// in `push_policy.rs`): `{request_id, thread_id?}` for approvals,
/// `{thread_id}` for messages, `{job_id}` for jobs. This delegate never reads
/// message text or tool names from a push — the Notification Service Extension
/// has already rewritten the visible title/body locally if it could reach the
/// gateway.
///
/// Category split (D-K3 / D-N3): low-risk approvals get inline Approve/Deny
/// actions; high-risk approvals get an "Open" action only and always deep-link
/// into the app so the approval clears the Face ID gate there.
final class PushCoordinator: NSObject, UNUserNotificationCenterDelegate, @unchecked Sendable {
    static let messageCategory = "THINCLAW_MESSAGE"
    /// Back-compat base approval category (used by callers that route on the
    /// approval family). Live pushes use the risk-split categories below.
    static let approvalCategory = "THINCLAW_APPROVAL"
    static let approvalLowCategory = "THINCLAW_APPROVAL_LOW"
    static let approvalHighCategory = "THINCLAW_APPROVAL_HIGH"
    static let jobCategory = "THINCLAW_JOB"

    static let approveActionID = "THINCLAW_APPROVE"
    static let denyActionID = "THINCLAW_DENY"
    static let openActionID = "THINCLAW_OPEN"

    private let dependencies: AppDependencies
    private let router: AppRouter

    @MainActor init(dependencies: AppDependencies, router: AppRouter) {
        self.dependencies = dependencies
        self.router = router
    }

    /// Register this coordinator as the notification-center delegate and install
    /// the category set. Called once at launch.
    @MainActor func configure() {
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        center.setNotificationCategories(Self.categories())
    }

    /// The four notification categories. Only the low-risk approval category
    /// carries inline Approve/Deny buttons; the high-risk one offers "Open"
    /// (deep-link → Face ID in-app) so a high-risk tool is never approved from
    /// the lock screen (D-K3).
    @MainActor static func categories() -> Set<UNNotificationCategory> {
        let approve = UNNotificationAction(
            identifier: approveActionID,
            title: "Approve",
            options: [.authenticationRequired])
        let deny = UNNotificationAction(
            identifier: denyActionID,
            title: "Deny",
            options: [.destructive])
        let open = UNNotificationAction(
            identifier: openActionID,
            title: "Open",
            options: [.foreground])

        let message = UNNotificationCategory(
            identifier: messageCategory,
            actions: [],
            intentIdentifiers: [],
            options: [])
        let approvalLow = UNNotificationCategory(
            identifier: approvalLowCategory,
            actions: [approve, deny],
            intentIdentifiers: [],
            options: [])
        let approvalHigh = UNNotificationCategory(
            identifier: approvalHighCategory,
            actions: [open],
            intentIdentifiers: [],
            options: [])
        let job = UNNotificationCategory(
            identifier: jobCategory,
            actions: [],
            intentIdentifiers: [],
            options: [])
        return [message, approvalLow, approvalHigh, job]
    }

    // MARK: - UNUserNotificationCenterDelegate

    /// Foreground presentation: still show the banner so an approval waiting
    /// while the user is in the app is not silently dropped. (Live Activity
    /// suppression is a gateway-side decision; by the time a push reaches here it
    /// was already deemed worth delivering.)
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound, .list]
    }

    /// The user tapped the notification or one of its actions. Route by action:
    /// low-risk Approve/Deny POST directly; everything else (default tap, Open)
    /// deep-links into the app via ``AppRouter``.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let ids = PushIDs(userInfo: response.notification.request.content.userInfo)

        await handle(
            ids: ids,
            actionIdentifier: response.actionIdentifier,
            category: response.notification.request.content.categoryIdentifier)
    }

    @MainActor
    private func handle(ids: PushIDs, actionIdentifier: String, category: String) async {
        AppLog.push.debug("Handling content-free notification action")
        switch actionIdentifier {
        case Self.approveActionID:
            await submitApproval(ids, action: "approve")
        case Self.denyActionID:
            await submitApproval(ids, action: "deny")
        default:
            // Default tap or the high-risk "Open" action: deep-link so the
            // in-app surface (with its Face ID gate) handles it.
            if let url = ids.deepLink(category: category) {
                router.handle(deepLink: url)
            }
        }
    }

    /// POST a low-risk approval decision over the pinned client (D-N3: inline
    /// actions are offered only for low-risk categories, enforced again here by
    /// only wiring Approve/Deny to `THINCLAW_APPROVAL_LOW`). Best-effort; the
    /// POST is idempotent by `request_id` server-side.
    @MainActor
    private func submitApproval(_ ids: PushIDs, action: String) async {
        guard let requestID = ids.requestID, let client = dependencies.makePushClient() else {
            return
        }
        do {
            _ = try await client.chatApprovalHandler(
                body: .json(.init(action: action, requestId: requestID, threadId: ids.threadID)))
        } catch {
            AppLog.push.error("Notification approval submission failed")
        }
        // The authoritative pull snapshot reconciles repeated/cross-device
        // decisions, including a request that desktop resolved first.
        if let approvals = dependencies.makeApprovalsStore() {
            await approvals.refresh()
        }
    }
}

/// The id-only payload the gateway ships under the `tc` dict (D-N1). Every field
/// is optional because a given category only carries the ids it needs.
private struct PushIDs: Sendable {
    let requestID: String?
    let threadID: String?
    let jobID: String?

    init(userInfo: [AnyHashable: Any]) {
        let tc = userInfo["tc"] as? [String: Any]
        requestID = tc?["request_id"] as? String
        threadID = tc?["thread_id"] as? String
        jobID = tc?["job_id"] as? String
    }

    /// The `thinclaw://` deep link for a default tap, chosen by category:
    /// approvals → `approval/<request_id>?thread=…`, jobs → `job/<job_id>`,
    /// messages → `thread/<thread_id>`.
    func deepLink(category: String) -> URL? {
        switch category {
        case PushCoordinator.approvalLowCategory,
            PushCoordinator.approvalHighCategory,
            PushCoordinator.approvalCategory:
            guard let requestID else { return nil }
            return AppRoute.approvals(requestID: requestID, threadID: threadID).url
        case PushCoordinator.jobCategory:
            return AppRoute.job(jobID).url
        default:
            return AppRoute.thread(threadID).url
        }
    }
}
