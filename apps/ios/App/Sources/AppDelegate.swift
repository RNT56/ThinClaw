import Foundation
import UserNotifications

#if canImport(UIKit)
    import UIKit
#endif

/// UIKit application delegate, bridged into the SwiftUI `App` via
/// `@UIApplicationDelegateAdaptor`, owning the two lifecycle hooks SwiftUI does
/// not expose: APNs device-token registration and the silent
/// (`content-available`) push handoff to background refresh.
///
/// It is deliberately thin — the token round-trip and notification routing live
/// in ``PushCoordinator`` / ``AppDependencies`` — but registration must be
/// driven from `UIApplicationDelegate` callbacks, so this shell forwards them.
///
/// The delegate holds no state of its own; the SwiftUI `App` injects the
/// composition root and the notification coordinator after they are built so
/// every effectful call goes through the shared, pinned graph
/// (docs/MOBILE_SECURITY.md D-N1 / D-X2).
@MainActor
final class AppDelegate: NSObject {
    /// Injected by ``ThinClawApp`` once the graph exists. Weak-free strong refs
    /// are fine: the delegate lives for the whole process, same as these.
    var dependencies: AppDependencies?
    var pushCoordinator: PushCoordinator?

    #if canImport(UIKit)
        /// Ask for notification authorization (alerts + sounds + badges) and, if
        /// granted, register for remote notifications. Called on launch while
        /// paired. The authorization prompt is idempotent — the system shows it
        /// only once, then returns the prior decision.
        func requestPushAuthorizationAndRegister() {
            Task {
                let center = UNUserNotificationCenter.current()
                let granted =
                    (try? await center.requestAuthorization(options: [
                        .alert, .sound, .badge,
                    ])) ?? false
                guard granted else { return }
                UIApplication.shared.registerForRemoteNotifications()
            }
        }
    #endif
}

#if canImport(UIKit)
    extension AppDelegate: UIApplicationDelegate {
        /// APNs handed us a device token: forward it (hex-encoded, per the
        /// `RegisterPushRequest.apns_token` contract) to the gateway over the
        /// pinned client.
        func application(
            _ application: UIApplication,
            didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
        ) {
            let hex = deviceToken.map { String(format: "%02x", $0) }.joined()
            guard let dependencies else { return }
            Task { await dependencies.registerPush(apnsToken: hex) }
        }

        /// APNs registration failed (no network, no entitlement, simulator
        /// without a paired push service). Non-fatal — we simply have no push
        /// token this launch and rely on the in-app stream + background refresh.
        func application(
            _ application: UIApplication,
            didFailToRegisterForRemoteNotificationsWithError error: any Error
        ) {
            // Intentionally silent (no body/URL logging, D-N1 hygiene). A later
            // launch re-attempts registration.
        }

        /// A silent (`content-available: 1`) push woke the app in the
        /// background: refresh the App Group snapshots and reload widgets, then
        /// report whether new data arrived so the system schedules future wakes
        /// sensibly.
        func application(
            _ application: UIApplication,
            didReceiveRemoteNotification userInfo: [AnyHashable: Any]
        ) async -> UIBackgroundFetchResult {
            guard let dependencies else { return .noData }
            return await BackgroundRefresh.handleSilentPush(dependencies: dependencies)
        }
    }
#endif
