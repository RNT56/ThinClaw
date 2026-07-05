import BackgroundTasks
import Foundation

#if canImport(UIKit)
    import UIKit
#endif

#if canImport(WidgetKit)
    import WidgetKit
#endif

/// Snapshot refresh: the App Group snapshots (status, approvals, jobs) that keep
/// widgets honest even without a foreground app.
///
/// Two triggers feed the same refresh:
/// - **Silent pushes** (`content-available: 1`, wired here at M2): the gateway
///   nudges the app to re-fetch when something changed, so widgets update
///   promptly without shipping content through APNs (docs/MOBILE_SECURITY.md
///   D-N1).
/// - **`BGAppRefresh`** (the slow safety net, wired at M3): a periodic fallback
///   for when no push arrived.
///
/// The concrete fetch → SnapshotKit write pipeline is owned by M3; M2 wires the
/// silent-push entry point and the widget reload so the plumbing exists and the
/// snapshot mapping can drop in without touching the delegate.
enum BackgroundRefresh {
    static let taskIdentifier = "com.thinclaw.ios.refresh"

    static func register() {
        // M3: BGTaskScheduler.shared.register(forTaskWithIdentifier:) →
        // handleSilentPush(dependencies:) on the periodic cadence.
    }

    /// Handle a silent (`content-available`) push: refresh the shared snapshots
    /// and reload widget timelines. Returns whether new data was produced so the
    /// caller can report the right `UIBackgroundFetchResult`.
    ///
    /// M2 reloads widgets unconditionally (cheap, and a silent push means the
    /// gateway believes something changed). The snapshot re-fetch that decides
    /// `.newData` vs `.noData` lands with M3's fetch pipeline; until then this
    /// reports `.newData` so the system keeps honoring the wake budget.
    @MainActor
    @discardableResult
    static func handleSilentPush(dependencies: AppDependencies) async -> BackgroundFetchOutcome {
        // M3: await dependencies.refreshSnapshots() to write SnapshotKit files.
        reloadWidgets()
        return .newData
    }

    /// Reload every widget timeline from the shared snapshots.
    @MainActor
    static func reloadWidgets() {
        #if canImport(WidgetKit)
            WidgetCenter.shared.reloadAllTimelines()
        #endif
    }
}

#if canImport(UIKit)
    /// Bridge to `UIBackgroundFetchResult` without leaking UIKit into callers on
    /// platforms that lack it.
    typealias BackgroundFetchOutcome = UIBackgroundFetchResult
#else
    enum BackgroundFetchOutcome {
        case newData, noData, failed
    }
#endif
