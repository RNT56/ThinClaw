import Foundation
import SwiftUI
import ThinClawCore

enum AppTab: Hashable {
    case chat, sessions, jobs, settings
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
    var settingsPath = NavigationPath()
    var showsApprovals = false

    /// The thread currently driving the Chat tab. Selecting a row in Sessions
    /// sets this and switches to Chat; a `thinclaw://thread/<id>` deep link does
    /// the same. Nil means the default assistant thread.
    var selectedThread: ThreadID?

    /// The approval a `thinclaw://approval/<request_id>` deep link (notification
    /// tap or high-risk "Open" action) asked the app to focus. The approvals
    /// surface reads this to scroll to and — for high-risk — Face ID-gate that
    /// specific request. Nil when the sheet was opened without a target.
    private(set) var focusedApprovalID: String?

    /// The job a `thinclaw://job/<job_id>` deep link asked the app to focus, for
    /// the Jobs surface to select. Nil for a bare `thinclaw://job`.
    private(set) var focusedJobID: String?

    /// Select a thread and focus the Chat tab (Sessions row tap / deep link).
    func openThread(_ id: ThreadID) {
        selectedThread = id
        selectedTab = .chat
    }

    func handle(deepLink url: URL) {
        guard url.scheme == "thinclaw" else { return }
        switch url.host() {
        case "thread":
            // `thinclaw://thread/<thread-id>` focuses that thread; a bare
            // `thinclaw://thread` just switches to the Chat tab.
            if let id = url.pathComponents.dropFirst().first, !id.isEmpty {
                openThread(ThreadID(id))
            } else {
                selectedTab = .chat
            }
        case "approval":
            // `thinclaw://approval/<request_id>?thread=<id>` focuses one pending
            // approval (notification tap / high-risk "Open"); a bare
            // `thinclaw://approval` just opens the sheet. The `thread` query is
            // carried so the in-app approval POST can attach `thread_id`.
            if let requestID = url.pathComponents.dropFirst().first, !requestID.isEmpty {
                focusedApprovalID = requestID
                if let thread = URLComponents(url: url, resolvingAgainstBaseURL: false)?
                    .queryItems?.first(where: { $0.name == "thread" })?.value, !thread.isEmpty
                {
                    selectedThread = ThreadID(thread)
                }
            } else {
                focusedApprovalID = nil
            }
            showsApprovals = true
        case "job":
            // `thinclaw://job/<job-id>` focuses that job; a bare `thinclaw://job`
            // just switches to the Jobs tab.
            focusedJobID = url.pathComponents.dropFirst().first.flatMap { $0.isEmpty ? nil : $0 }
            selectedTab = .jobs
        case "quick-ask":
            selectedTab = .chat
        default:
            break
        }
    }
}
