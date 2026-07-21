import Foundation
import Observation
import SwiftUI
import ThinClawAuth
import ThinClawWidgetKitShared

#if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
    import ThinClawWatchBridge
#endif

#if canImport(UIKit)
    import UIKit
#endif

/// The application-lifetime composition owner. Feature stores live under its
/// dependency graph and are rebuilt only when the authenticated gateway
/// changes; routing and platform relays share that same context.
@MainActor
@Observable
final class AppCoordinator {
    let dependencies: AppDependencies
    let router: AppRouter
    let push: PushCoordinator
    var pendingGatewayReplacementURL: URL?

    #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
        let watchProvisioning: WatchProvisioning
    #endif

    init() {
        let dependencies = AppDependencies()
        let router = AppRouter()
        self.dependencies = dependencies
        self.router = router
        self.push = PushCoordinator(dependencies: dependencies, router: router)
        #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
            self.watchProvisioning = WatchProvisioning()
        #endif
    }

    func configure(appDelegate: AppDelegate?) {
        #if canImport(Security)
            try? DeviceUnlockProbe.provision()
        #endif
        push.configure()
        #if canImport(UIKit)
            appDelegate?.dependencies = dependencies
            appDelegate?.pushCoordinator = push
        #endif
        #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
            dependencies.onSnapshotsPublished = { [watchProvisioning] status, approvals in
                watchProvisioning.mirror(status: status, approvals: approvals)
            }
        #endif
    }

    func handleOpenURL(_ url: URL) {
        if case .pair = AppRoute(url: url) {
            if dependencies.isPaired {
                AppLog.pairing.notice("Pairing link requires replacement confirmation")
                pendingGatewayReplacementURL = url
            } else {
                AppLog.pairing.notice("Routing external pairing link to onboarding")
                dependencies.handlePairingURL(url)
            }
            return
        }
        router.handle(deepLink: url)
    }

    func replaceGateway() async {
        guard let url = pendingGatewayReplacementURL else { return }
        pendingGatewayReplacementURL = nil
        await dependencies.unpair()
        AppLog.pairing.notice("Starting replacement gateway pairing")
        dependencies.handlePairingURL(url)
    }

    func sceneDidChange(to phase: ScenePhase, appDelegate: AppDelegate?) {
        switch phase {
        case .active:
            Task { await dependencies.startSessionIfPaired() }
            #if canImport(UIKit)
                if dependencies.isPaired {
                    appDelegate?.requestPushAuthorizationAndRegister()
                }
            #endif
            #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
                if dependencies.isPaired {
                    AppLog.watchRelay.debug("Activating paired watch relay")
                    watchProvisioning.activateIfPaired()
                }
            #endif
        case .background:
            Task { await dependencies.stopSession() }
            #if canImport(UIKit)
                BackgroundRefresh.scheduleAppRefresh()
            #endif
        default:
            break
        }
    }

    func pairingStateDidChange(_ paired: Bool) {
        #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
            if paired {
                AppLog.watchRelay.debug("Pairing state activated watch relay")
                watchProvisioning.activateIfPaired()
            } else {
                AppLog.watchRelay.debug("Pairing state deprovisioned watch relay")
                Task { await watchProvisioning.deprovisionAndTearDown() }
            }
        #endif
    }
}
