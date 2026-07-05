import BackgroundTasks
import Foundation

/// BGAppRefresh registration: the slow safety net that refreshes the
/// App Group snapshots (status, approvals, jobs) so widgets stay honest even
/// without pushes. Wired at M3.
enum BackgroundRefresh {
    static let taskIdentifier = "com.thinclaw.ios.refresh"

    static func register() {
        // M3: BGTaskScheduler.shared.register(forTaskWithIdentifier:) →
        // fetch /api/gateway/status + /api/chat/approvals + /api/jobs/summary,
        // write SnapshotKit files, WidgetCenter.reloadTimelines.
    }
}
