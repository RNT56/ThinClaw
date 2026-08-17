import Foundation
import Observation
import SwiftUI
import ThinClawAuth
import ThinClawCore
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

    @ObservationIgnored private var currentScenePhase: ScenePhase = .inactive
    @ObservationIgnored private var isConfigured = false
    @ObservationIgnored private weak var appDelegate: AppDelegate?
    @ObservationIgnored private var lifecycleReconciler: ForegroundLifecycleReconciler!

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
        self.lifecycleReconciler = ForegroundLifecycleReconciler(
            actions: .init(
                activate: { [weak dependencies] in
                    await dependencies?.startSessionIfPaired()
                },
                didActivate: { [weak self] in
                    self?.didActivateCurrentPairing()
                },
                deactivate: { [weak dependencies] in
                    await dependencies?.stopSession()
                }))
    }

    func configure(appDelegate: AppDelegate?, initialScenePhase: ScenePhase) {
        self.appDelegate = appDelegate
        currentScenePhase = initialScenePhase
        isConfigured = true
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
            dependencies.onWillUnpair = { [watchProvisioning] in
                await watchProvisioning.deprovisionAndTearDown()
            }
            watchProvisioning.resumePendingWipes()
        #endif
        reconcileLifecycle()
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
        self.appDelegate = appDelegate
        let previousPhase = currentScenePhase
        currentScenePhase = phase
        if phase == .background, previousPhase != .background {
            #if canImport(UIKit)
                BackgroundRefresh.scheduleAppRefresh()
            #endif
        }
        reconcileLifecycle()
    }

    func pairingStateDidChange(_ paired: Bool) {
        // `isPaired` is the authoritative state. The argument is retained for a
        // readable SwiftUI observation hook and guarded in debug builds.
        assert(paired == dependencies.isPaired)
        reconcileLifecycle()
        #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
            if !paired {
                AppLog.watchRelay.debug("Pairing state retained pending Watch wipe delivery")
                watchProvisioning.resumePendingWipes()
            }
        #endif
    }

    /// Coalesce launch, scene, and pairing observations into one ordered desired
    /// state. The reconciler serialises suspended start/stop operations and
    /// generation-checks activation before push/Watch effects are published.
    private func reconcileLifecycle() {
        let target: ForegroundLifecycleReconciler.Target
        if isConfigured, currentScenePhase == .active, dependencies.isPaired {
            target = .active(pairingGeneration: dependencies.pairingGeneration)
        } else {
            target = .inactive
        }
        lifecycleReconciler.reconcile(to: target)
    }

    private func didActivateCurrentPairing() {
        guard currentScenePhase == .active, dependencies.isPaired else { return }
        #if canImport(UIKit)
            appDelegate?.requestPushAuthorizationAndRegister()
        #endif
        #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
            AppLog.watchRelay.debug("Activating paired watch relay")
            watchProvisioning.activateIfPaired()
        #endif
    }
}
