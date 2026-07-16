import Foundation
import SwiftUI
import ThinClawCore
import ThinClawWidgetKitShared

enum AppTab: Hashable {
    case chat, sessions, approvals, jobs, settings
}

/// Owns tab selection and each tab's navigation path so deep links
/// (`thinclaw://thread|approval|job|quick-ask`) from widgets, notifications,
/// and the watch can drive navigation programmatically.
@MainActor
@Observable
final class AppRouter {
    var selectedTab: AppTab = .chat
    var chatPath = NavigationPath()
    var sessionsPath = NavigationPath()
    var jobsPath = NavigationPath()
    var approvalsPath = NavigationPath()
    var settingsPath = NavigationPath()

    /// The thread currently driving the Chat tab. Selecting a row in Sessions
    /// sets this and switches to Chat; a `thinclaw://thread/<id>` deep link does
    /// the same. Nil means the default assistant thread.
    var selectedThread: ThreadID?

    /// The approval a `thinclaw://approval/<request_id>` deep link (notification
    /// tap or high-risk "Open" action) asked the app to focus. The approvals
    /// surface reads this to scroll to and — for high-risk — Face ID-gate that
    /// specific request. Nil when the sheet was opened without a target.
    var focusedApprovalID: String?

    /// The job a `thinclaw://job/<job_id>` deep link asked the app to focus, for
    /// the Jobs surface to select. Nil for a bare `thinclaw://job`.
    var focusedJobID: String?

    /// Select a thread and focus the Chat tab (Sessions row tap / deep link).
    func openThread(_ id: ThreadID) {
        selectedThread = id
        selectedTab = .chat
    }

    func handle(deepLink url: URL) {
        guard let route = AppRoute(url: url) else { return }
        switch route {
        case .pair:
            break  // Pairing is routed to OnboardingStore by the app coordinator.
        case .thread(let id):
            if let id {
                openThread(ThreadID(id))
            } else {
                selectedTab = .chat
            }
        case .approvals(let requestID, let threadID):
            focusedApprovalID = requestID
            if let threadID { selectedThread = ThreadID(threadID) }
            selectedTab = .approvals
        case .job(let id):
            focusedJobID = id
            selectedTab = .jobs
        case .quickAsk:
            selectedTab = .chat
        }
    }
}
