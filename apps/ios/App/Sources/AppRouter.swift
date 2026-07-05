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

    func handle(deepLink url: URL) {
        guard url.scheme == "thinclaw" else { return }
        switch url.host() {
        case "thread":
            selectedTab = .chat
        case "approval":
            showsApprovals = true
        case "job":
            selectedTab = .jobs
        case "quick-ask":
            selectedTab = .chat
        default:
            break
        }
        // M2: route path components (thread id, request id) into the
        // matching NavigationPath / store.
    }
}
