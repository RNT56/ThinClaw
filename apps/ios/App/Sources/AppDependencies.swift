import FeatureApprovals
import FeatureChat
import FeatureJobs
import FeatureOnboarding
import FeatureSessions
import FeatureSettings
import Foundation
import OpenAPIRuntime
import OpenAPIURLSession
import SwiftUI
import ThinClawAPI
import ThinClawAuth
import ThinClawCore
import ThinClawLiveActivity
import ThinClawPersistence
import ThinClawSnapshotKit
import ThinClawTransport
import ThinClawWidgetKitShared

#if canImport(UIKit)
    import UIKit
#endif

/// Composition root: builds the real dependency graph once at launch and
/// hands it down via the SwiftUI environment. Every effectful boundary is a
/// protocol so features and tests can inject fakes.
///
/// ## M1 production graph
/// When a device credential is present, this assembles the live gateway stack:
/// Keychain credential → ``GatewayEndpoint`` + a **pinned** `URLSession`
/// (`PinnedSessionDelegate`, D-X2) → the byte-stream provider (event SSE) and
/// the generated REST client transport (both over the *same* pinned session, so
/// nothing bypasses TLS pinning) → ``GatewaySession`` → the chat/sessions
/// stores. The session is started/stopped from `scenePhase`.
@MainActor
@Observable
final class AppDependencies {
    let transcriptStore: any TranscriptStoring

    /// Shared-group keychain holding the device credential (D-K1/D-K2).
    private let keychain: any KeychainStoring

    /// Whether a device credential is present — drives RootView between
    /// onboarding and the tab shell. Recomputed from the Keychain at launch and
    /// whenever pairing/unpairing changes it.
    private(set) var isPaired: Bool

    /// The live gateway session, built lazily from the stored credential the
    /// first time it is needed while paired. Nil before pairing (or after
    /// unpair).
    private(set) var session: GatewaySession?

    /// The single shared approvals store, built lazily on first use while
    /// paired. Shared so the badge count and the presented sheet observe the
    /// same pending set. Cleared on unpair.
    private var approvalsStore: ApprovalsStore?

    /// The App Group snapshot pipeline (status/approvals/jobs → widgets). Built
    /// once and reused; a `nil` container inside makes every call a no-op, so it
    /// is always safe to touch. Owns its own publisher, independent of the live
    /// event session's lifecycle.
    private let snapshotService = SnapshotService()

    /// Background task mirroring the approvals store's pending set into the
    /// snapshot publisher; cancelled on unpair.
    private var snapshotMirrorTask: Task<Void, Never>?

    /// Sink for pushing the freshest glanceable snapshots (status + approvals) to
    /// a paired watch on a significant change. Set by the app once at launch to
    /// `WatchProvisioning.mirror`; `nil` on hosts without WatchConnectivity, where
    /// it is simply never invoked. The snapshots are read back from the iOS App
    /// Group (already content-minimised by ``SnapshotPrivacyPolicy``) so the watch
    /// mirror carries exactly what the widgets show.
    var onSnapshotsPublished: (@MainActor (AgentStatusSnapshot, PendingApprovalsSnapshot) -> Void)?

    #if canImport(ActivityKit)
        /// The agent-run Live Activity manager, built lazily from the paired
        /// session the first time a thread is observed. Owns at most one activity
        /// per thread; nil before pairing (or after unpair/teardown).
        private var liveActivityManager: LiveActivityManager?
        /// The ActivityKit controller the manager drives, retained here so its
        /// weak `manager` back-reference (for push-token forwarding) stays valid.
        private var liveActivityController: LiveActivityKitController?
    #endif

    init(
        transcriptStore: any TranscriptStoring = AppDependencies.defaultTranscriptStore(),
        keychain: any KeychainStoring = AppDependencies.defaultKeychain()
    ) {
        self.transcriptStore = transcriptStore
        self.keychain = keychain
        let existing = (try? DeviceCredential.load(from: keychain)) ?? nil
        self.isPaired = existing != nil
    }

    /// The production transcript cache: the GRDB-backed store at the default
    /// app-support location (app-process-only). Falls back to the in-memory
    /// store if the database cannot be opened, so a storage fault degrades to a
    /// non-persistent cache rather than blocking chat entirely.
    static func defaultTranscriptStore() -> any TranscriptStoring {
        (try? GRDBTranscriptStore.atDefaultLocation()) ?? InMemoryTranscriptStore()
    }

    /// The real shared-group keychain store. The resolved access group is
    /// `<AppIdentifierPrefix>com.thinclaw.shared`; the prefix is only knowable
    /// from the entitlement at runtime, so we let SecItem default to the app's
    /// first entitled group by passing `nil` here until the Tuist target wires
    /// the resolved string. Widgets/extensions read the same item via the
    /// shared entitlement.
    static func defaultKeychain() -> any KeychainStoring {
        #if canImport(Security)
            return SecItemKeychainStore()
        #else
            return InMemoryKeychain()
        #endif
    }

    // MARK: - Session lifecycle (scenePhase)

    /// Ensure the live session exists (building it from the credential) and
    /// start its event stream. Called on `.active` while paired. Idempotent.
    func startSessionIfPaired() async {
        guard isPaired else { return }
        let session = ensureSession()
        await session?.start()
        // Mirror approvals into the snapshot pipeline for the foreground live
        // path, and kick one immediate fetch so widgets reflect current state
        // right after launch/foreground without waiting for a push.
        startSnapshotMirroring()
        await refreshSnapshots()
    }

    /// Tear down the live event stream when the app backgrounds. The session
    /// object is retained so a later `.active` restarts it without rebuilding.
    ///
    /// The Live Activity manager is *not* stopped here: a run in flight keeps its
    /// activity alive across a background (the gateway pushes updates to the
    /// per-activity token while the app is asleep). It is torn down only on
    /// unpair.
    func stopSession() async {
        await session?.shutdown()
    }

    // MARK: - Snapshot pipeline (M3)

    /// Fetch a fresh App Group snapshot (gateway status + approvals + jobs) over
    /// the pinned client and write it, then reload widget timelines. Called from
    /// the silent-push handler, the `BGAppRefresh` task, and on foreground.
    /// Returns whether a snapshot was produced so the background caller can pick
    /// the right `UIBackgroundFetchResult`. No-op (returns `false`) when
    /// unpaired or when the App Group container is unavailable.
    @discardableResult
    func refreshSnapshots() async -> Bool {
        guard isPaired, let client = makePushClient() else { return false }
        let wrote = await snapshotService.refresh(client: client)
        if wrote { pushWatchMirror() }
        return wrote
    }

    /// Push the freshest glanceable snapshots to a paired watch, if a sink is
    /// wired. Reads the two snapshots the publisher just wrote back from the iOS
    /// App Group (already content-minimised) so the watch mirror carries exactly
    /// what the widgets show. No-op when no sink is set (no watch / unsupported
    /// host) or when no snapshot has been written yet.
    private func pushWatchMirror() {
        guard let sink = onSnapshotsPublished,
            let store = WidgetSnapshotAccess.store()
        else { return }
        let status = (try? store.load(AgentStatusSnapshot.self)) ?? nil
        let approvals = (try? store.load(PendingApprovalsSnapshot.self)) ?? nil
        guard status != nil || approvals != nil else { return }
        sink(
            status ?? AgentStatusSnapshot(generatedAt: .now, phase: .idle),
            approvals ?? PendingApprovalsSnapshot(generatedAt: .now, approvals: []))
    }

    /// Begin mirroring the shared approvals store's pending set into the snapshot
    /// publisher so the approvals widget tracks the app live while foregrounded.
    /// Idempotent; cold-loads the store and then folds each change. The status
    /// widget's `waitingForApproval` phase falls out of this without a separate
    /// event tap (the publisher promotes an idle phase when approvals are
    /// non-empty).
    func startSnapshotMirroring() {
        guard isPaired, snapshotMirrorTask == nil, let store = makeApprovalsStore() else {
            return
        }
        snapshotMirrorTask = Task { [weak self] in
            await store.start()
            // Observe `pending` and re-publish on every change until cancelled.
            while !Task.isCancelled {
                let pending = store.pending
                await self?.snapshotService.mirror(approvals: pending)
                // Mirror the same significant change to a paired watch.
                self?.pushWatchMirror()
                await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
                    withObservationTracking {
                        _ = store.pending
                    } onChange: {
                        continuation.resume()
                    }
                }
            }
        }
    }

    /// Stop mirroring approvals into the snapshot publisher.
    func stopSnapshotMirroring() {
        snapshotMirrorTask?.cancel()
        snapshotMirrorTask = nil
    }

    /// Build the ``GatewaySession`` from the stored credential if not already
    /// built. Returns nil if there is no credential or no usable base URL.
    @discardableResult
    private func ensureSession() -> GatewaySession? {
        if let session { return session }
        guard let credential = (try? DeviceCredential.load(from: keychain)) ?? nil,
            let baseURL = credential.preferredBaseURL
        else { return nil }

        let token = credential.deviceToken
        let tokenProvider: @Sendable () -> String? = { token }

        // ONE pinned session, shared by the event byte stream and the REST
        // transport, so both go through TLS pinning + the D-X2 policy. There is
        // no unpinned default anywhere in this graph.
        let pinnedSession = PinnedSessionDelegate(
            pinnedFingerprint: credential.serverFingerprint
        ).makeSession()

        let provider = URLSessionByteStreamProvider(baseURL: baseURL, session: pinnedSession)
        let stream = GatewayStream(provider: provider, token: tokenProvider)

        let transport = URLSessionTransport(configuration: .init(session: pinnedSession))
        let client = GatewayClient.make(
            baseURL: baseURL, token: tokenProvider, transport: transport)

        let session = GatewaySession(client: client, stream: stream)
        self.session = session
        return session
    }

    // MARK: - Push registration (M2)

    /// A generated REST ``Client`` over the **same** pinned session policy as the
    /// live session, built directly from the stored credential. Used by
    /// ``PushCoordinator`` to register/clear the APNs token and to action
    /// low-risk approvals from a notification without going through a chat store.
    /// Returns `nil` when unpaired or when no policy-allowed URL is available.
    ///
    /// This does not reuse ``ensureSession``'s client because push registration
    /// must work on a cold launch triggered by APNs before the event stream is
    /// started, and low-risk approve/deny actions fire from the notification
    /// delegate independent of any open thread.
    func makePushClient() -> Client? {
        guard let credential = (try? DeviceCredential.load(from: keychain)) ?? nil,
            let baseURL = credential.preferredBaseURL
        else { return nil }
        let token = credential.deviceToken
        let pinnedSession = PinnedSessionDelegate(
            pinnedFingerprint: credential.serverFingerprint
        ).makeSession()
        return GatewayClient.make(baseURL: baseURL, token: { token }, session: pinnedSession)
    }

    /// Register `apnsToken` (hex) with the gateway for content-free pushes
    /// (`PUT /api/devices/me/push`, D-N1). `environment` is `"development"` in
    /// DEBUG builds (sandbox APNs) and `"production"` otherwise. Best-effort:
    /// failures are swallowed so a transient gateway outage does not crash the
    /// app on launch; the token is re-sent on the next registration.
    func registerPush(apnsToken: String) async {
        guard let client = makePushClient() else { return }
        let environment: String
        #if DEBUG
            environment = "development"
        #else
            environment = "production"
        #endif
        _ = try? await client.devicesMePushRegisterHandler(
            body: .json(.init(apnsToken: apnsToken, environment: environment)))
    }

    // MARK: - Live Activity (M3)

    #if canImport(ActivityKit)
        /// Build (once) the ``LiveActivityManager`` over the paired session: its
        /// events feed the run tracker, an ``LiveActivityKitController`` performs
        /// the real ActivityKit calls, and a ``GatewayLiveActivityRegistrar`` over
        /// the pinned client registers per-activity + push-to-start tokens. Nil
        /// when unpaired or when no policy-allowed client is available.
        private func ensureLiveActivityManager() -> LiveActivityManager? {
            if let liveActivityManager { return liveActivityManager }
            guard let session = ensureSession(), let client = makePushClient() else { return nil }

            let controller = LiveActivityKitController()
            let manager = LiveActivityManager(
                eventSource: GatewaySessionEventSource(session: session),
                controller: controller,
                registrar: GatewayLiveActivityRegistrar(client: client))
            // The controller forwards new per-activity push tokens back to the
            // manager for gateway registration; wire the weak back-reference.
            controller.manager = manager

            liveActivityController = controller
            liveActivityManager = manager
            return manager
        }

        /// Start driving the agent-run Live Activity for `thread`: observe its
        /// events and register the device's push-to-start token so a killed app
        /// can be spawned by the gateway. Called when the Chat tab resolves its
        /// active thread and on `.active` while paired. Idempotent per thread.
        func startLiveActivity(for thread: ThreadID, title: String) {
            guard isPaired, let manager = ensureLiveActivityManager() else { return }
            manager.observe(thread: thread, title: title)
            if let controller = liveActivityController {
                manager.startPushToStartRegistration(tokens: {
                    controller.pushToStartTokenUpdates()
                })
            }
        }

        /// Tear down the Live Activity manager: stop observation and end every
        /// activity. Called on unpair and full teardown.
        func stopLiveActivity() async {
            await liveActivityManager?.stop()
            liveActivityManager = nil
            liveActivityController = nil
        }
    #endif

    // MARK: - Store factories

    /// Build a chat store for `thread`, wired to the live session and the
    /// transcript cache. Requires a paired session.
    func makeChatStore(thread: ThreadID) -> ChatStore? {
        guard let session = ensureSession() else { return nil }
        return ChatStore(threadID: thread, session: session, store: transcriptStore)
    }

    /// Build the sessions-list store, wired to the live session and cache.
    func makeSessionsStore() -> SessionsStore? {
        guard let session = ensureSession() else { return nil }
        return SessionsStore(session: session, store: transcriptStore)
    }

    /// Build the M5 Settings store, wired to the paired credential: a
    /// device-management adapter over the pinned client
    /// (`GET /api/devices/me`, companions, revoke), the paired-gateway identity
    /// from the credential, the App Group defaults for notification-preview
    /// preferences (so the NSE reads the same values), a real Face ID gate for
    /// the connection-detail reveal (D-K3), the live connection-state stream, and
    /// an enhanced-protection control that persists the shared overlay preference
    /// and re-tags the transcript cache. Nil until paired.
    func makeSettingsStore() -> SettingsStore? {
        guard let credential = (try? DeviceCredential.load(from: keychain)) ?? nil,
            let client = makePushClient(),
            let session = ensureSession()
        else { return nil }

        let identity = PairedGatewayIdentity(credential: credential)
        let store = transcriptStore
        let protection = ClosureProtectionControl(
            current: UserDefaults.standard.bool(
                forKey: PrivacySettingsKey.enhancedProtection)
        ) { enabled in
            // Persist for the app-switcher redaction overlay's @AppStorage read…
            UserDefaults.standard.set(
                enabled, forKey: PrivacySettingsKey.enhancedProtection)
            // …and re-tag the transcript cache if it is the file-backed store.
            if let grdb = store as? GRDBTranscriptStore {
                return grdb.applyFileProtection(enhanced: enabled)
            }
            return false
        }

        return SettingsStore(
            devices: GatewayDeviceManager(client: client),
            identity: identity,
            biometrics: SettingsBiometricGate(),
            unpairer: ClosureUnpairing { [weak self] in await self?.unpair() },
            keyValueStore: AppGroupDefaultsStore(),
            protectionControl: protection,
            connectionSource: GatewaySessionConnectionSource(session: session))
    }

    /// Build the read-only Jobs glance store, wired to the generated REST client
    /// (list/summary/detail) and a hand-rolled pinned fetch for the per-job event
    /// tail (`GET /api/jobs/{id}/events`, which is not part of the generated
    /// surface). Over the **same** pinned-session policy as the rest of the graph
    /// (D-X2). Nil until paired. The phone token holds `jobs:read` only, so the
    /// resulting store is read-only by construction.
    func makeJobsStore() -> JobsStore? {
        guard let credential = (try? DeviceCredential.load(from: keychain)) ?? nil,
            let baseURL = credential.preferredBaseURL
        else { return nil }

        let token = credential.deviceToken
        let tokenProvider: @Sendable () -> String? = { token }
        let pinnedSession = PinnedSessionDelegate(
            pinnedFingerprint: credential.serverFingerprint
        ).makeSession()
        let client = GatewayClient.make(
            baseURL: baseURL, token: tokenProvider, session: pinnedSession)

        let adapter = GatewayJobsAdapter(
            client: client, baseURL: baseURL, token: tokenProvider, session: pinnedSession)
        return JobsStore(gateway: adapter)
    }

    /// The shared approvals store, wired to the live session and the real
    /// `LocalAuthentication` biometric gate (D-K3). Built once and reused so
    /// the badge and the sheet share one pending set. Nil until paired.
    func makeApprovalsStore() -> ApprovalsStore? {
        if let approvalsStore { return approvalsStore }
        guard let session = ensureSession() else { return nil }
        let store = ApprovalsStore(
            gateway: GatewaySessionApprovalsGateway(session: session),
            biometrics: LocalAuthenticationGate())
        approvalsStore = store
        return store
    }

    /// Resolve a default thread for the Chat tab when the user has not selected
    /// one: the gateway's pinned `assistant_thread` when available, otherwise
    /// the most-recently-updated cached thread, falling back to the first
    /// regular thread in the gateway's listing. Nil when there are no threads at
    /// all yet.
    ///
    /// The pinned assistant thread is now surfaced by
    /// `GatewaySession.threadListing()` (the OpenAPI spec models
    /// `assistant_thread` as an optional `$ref`, which the generated
    /// `ThreadListResponse` exposes as `ThreadInfo?`), so it is preferred as the
    /// landing thread ahead of arbitrary cached or regular threads.
    func defaultThread() async -> ThreadID? {
        // Best-effort listing: a failure (offline, unpaired) degrades to the
        // cached fallback below rather than throwing.
        var listing: ThreadListing?
        if let session = ensureSession() {
            listing = try? await session.threadListing()
        }
        if let assistant = listing?.assistantThread {
            return assistant.id
        }
        if let cached = try? await transcriptStore.threads(), let first = cached.first {
            return first.id
        }
        return listing?.threads.first?.id
    }

    // MARK: - Onboarding / unpair

    /// Build the onboarding store, wired to the live pairing service and this
    /// keychain; `onPaired` flips `isPaired` so RootView swaps to the shell.
    func makeOnboardingStore() -> OnboardingStore {
        OnboardingStore(
            pairingService: LivePairingService(),
            keychain: keychain,
            deviceName: Self.defaultDeviceName(),
            onPaired: { [weak self] _ in
                self?.isPaired = true
            })
    }

    /// Sign out: best-effort self-revoke on the gateway (POST
    /// `/api/devices/{id}/revoke`), then erase the local credential regardless
    /// of the network result, tear down the live session, and flip back to
    /// onboarding.
    func unpair() async {
        if let credential = (try? DeviceCredential.load(from: keychain)) ?? nil {
            // Clear the push registration first (needs the still-valid token),
            // then self-revoke. Both are best-effort — the local erase below is
            // authoritative for signing out.
            if let client = makePushClient() {
                _ = try? await client.devicesMePushRemoveHandler()
            }
            await UnpairService.revoke(credential)
        }
        try? DeviceCredential.erase(from: keychain)
        stopSnapshotMirroring()
        approvalsStore?.stop()
        approvalsStore = nil
        #if canImport(ActivityKit)
            await stopLiveActivity()
        #endif
        await session?.shutdown()
        session = nil
        isPaired = false
    }

    /// Default device name for the confirm sheet (D-P1): the user's device
    /// name, editable before pairing.
    private static func defaultDeviceName() -> String {
        #if canImport(UIKit)
            return UIDevice.current.name
        #else
            return "iPhone"
        #endif
    }
}
