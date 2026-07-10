import BackgroundTasks
import Foundation
import OSLog

#if canImport(UIKit)
    import UIKit
#endif

#if canImport(WidgetKit)
    import WidgetKit
#endif

/// Snapshot refresh: the App Group snapshots (status, approvals, jobs) that keep
/// widgets honest even without a foreground app.
///
/// Three triggers feed the same fetch → write → reload pipeline:
/// - **Foreground:** `AppDependencies.startSessionIfPaired()` kicks one refresh
///   and starts live approval mirroring.
/// - **Silent pushes** (`content-available: 1`, D-N1): the gateway nudges the
///   app to re-fetch when something changed, so widgets update promptly without
///   shipping content through APNs (docs/MOBILE_SECURITY.md D-N1).
/// - **`BGAppRefresh`** (the slow safety net): a periodic fallback for when no
///   push arrived, scheduled under the `com.thinclaw.ios.refresh` identifier
///   declared in `BGTaskSchedulerPermittedIdentifiers`.
///
/// The concrete fetch lives in ``AppDependencies/refreshSnapshots()``; this type
/// owns the OS-facing plumbing (task registration/scheduling and the silent-push
/// entry point) and the widget reload.
enum BackgroundRefresh {
    static let taskIdentifier = "com.thinclaw.ios.refresh"

    /// Minimum spacing between `BGAppRefresh` runs. The system treats this as a
    /// floor, not a guarantee — actual cadence depends on usage and power.
    static let minimumRefreshInterval: TimeInterval = 15 * 60
    private static let logger = Logger(
        subsystem: "com.thinclaw.ios", category: "background-refresh")
    private static let diagnostics = BackgroundRefreshDiagnostics()

    // MARK: - BGTaskScheduler wiring

    #if canImport(BackgroundTasks) && canImport(UIKit)
        /// Register the `BGAppRefresh` handler. Must be called **once**, before
        /// the app finishes launching (from `application(_:didFinishLaunching…)`
        /// or the SwiftUI launch `task`), or the scheduler traps. The handler
        /// runs the same fetch → write → reload pipeline as a silent push and
        /// re-arms the next refresh before returning.
        static func register(dependencies: @escaping @MainActor @Sendable () -> AppDependencies?) {
            BGTaskScheduler.shared.register(
                forTaskWithIdentifier: taskIdentifier, using: nil
            ) { task in
                guard let appRefresh = task as? BGAppRefreshTask else {
                    task.setTaskCompleted(success: false)
                    return
                }
                handle(appRefresh, dependencies: dependencies)
            }
        }

        /// Run one background refresh for a scheduled `BGAppRefreshTask`: re-arm
        /// the next one, run the fetch under an expiration guard, and report
        /// completion. Re-arming first ensures the chain continues even if the
        /// fetch is cut short.
        private static func handle(
            _ task: BGAppRefreshTask,
            dependencies: @escaping @MainActor @Sendable () -> AppDependencies?
        ) {
            scheduleAppRefresh()

            let completion = BackgroundTaskCompletion(task: task)
            let work = Task { @MainActor in
                let produced = await (dependencies()?.refreshSnapshots() ?? false)
                reloadWidgets()
                completion.finish(success: produced)
            }
            task.expirationHandler = {
                work.cancel()
                completion.finish(success: false)
            }
        }

        /// Submit the next `BGAppRefresh` request. Safe to call repeatedly; a
        /// resubmission replaces the pending request. Failures (e.g. the app is
        /// not authorized for background refresh) are swallowed — the live and
        /// silent-push paths still update widgets.
        static func scheduleAppRefresh() {
            let request = BGAppRefreshTaskRequest(identifier: taskIdentifier)
            request.earliestBeginDate = Date(timeIntervalSinceNow: minimumRefreshInterval)
            do {
                try BGTaskScheduler.shared.submit(request)
                Task { await diagnostics.recordSchedulingSuccess() }
            } catch {
                Task { await diagnostics.recordSchedulingFailure() }
                logger.error("Unable to schedule app refresh: \(String(describing: error), privacy: .public)")
            }
        }
    #else
        static func register(dependencies: @escaping @MainActor @Sendable () -> AppDependencies?) {}
        static func scheduleAppRefresh() {}
    #endif

    // MARK: - Silent push

    /// Handle a silent (`content-available`) push: refresh the shared snapshots
    /// over the pinned client and reload widget timelines. Returns whether new
    /// data was produced so the caller can report the right
    /// `UIBackgroundFetchResult`.
    @MainActor
    @discardableResult
    static func handleSilentPush(dependencies: AppDependencies) async -> BackgroundFetchOutcome {
        let produced = await dependencies.refreshSnapshots()
        reloadWidgets()
        return produced ? .newData : .noData
    }

    /// Reload every widget timeline from the shared snapshots.
    @MainActor
    static func reloadWidgets() {
        #if canImport(WidgetKit)
            WidgetCenter.shared.reloadAllTimelines()
        #endif
    }
}

actor BackgroundRefreshDiagnostics {
    private(set) var lastScheduledAt: Date?
    private(set) var lastSchedulingFailed = false

    func recordSchedulingSuccess() {
        lastScheduledAt = .now
        lastSchedulingFailed = false
    }

    func recordSchedulingFailure() {
        lastSchedulingFailed = true
    }
}

#if canImport(BackgroundTasks) && canImport(UIKit)
    private final class BackgroundTaskCompletion: @unchecked Sendable {
        private let lock = NSLock()
        private var task: BGTask?

        init(task: BGTask) {
            self.task = task
        }

        func finish(success: Bool) {
            let task = lock.withLock { () -> BGTask? in
                defer { self.task = nil }
                return self.task
            }
            task?.setTaskCompleted(success: success)
        }
    }
#endif

#if canImport(UIKit)
    /// Bridge to `UIBackgroundFetchResult` without leaking UIKit into callers on
    /// platforms that lack it.
    typealias BackgroundFetchOutcome = UIBackgroundFetchResult
#else
    enum BackgroundFetchOutcome {
        case newData, noData, failed
    }
#endif
