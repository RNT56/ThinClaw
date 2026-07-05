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
            showsApprovals = true
        case "job":
            selectedTab = .jobs
        case "quick-ask":
            selectedTab = .chat
        default:
            break
        }
    }
}
