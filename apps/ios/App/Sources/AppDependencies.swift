import FeatureApprovals
import FeatureChat
import FeatureOnboarding
import FeatureSessions
import Foundation
import OpenAPIRuntime
import OpenAPIURLSession
import SwiftUI
import ThinClawAPI
import ThinClawAuth
import ThinClawCore
import ThinClawPersistence
import ThinClawTransport

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
    }

    /// Tear down the live event stream when the app backgrounds. The session
    /// object is retained so a later `.active` restarts it without rebuilding.
    func stopSession() async {
        await session?.shutdown()
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
    /// one: the most-recently-updated cached thread, falling back to the
    /// gateway's listing. Nil when there are no threads at all yet.
    ///
    /// NOTE (concern for the API stage): the gateway's `assistant_thread` is not
    /// surfaced by `GatewaySession.threads()` because the committed OpenAPI
    /// snapshot models it as `oneOf: [null, $ref]`, which swift-openapi-generator
    /// drops from the generated `ThreadListResponse`. Until that spec pattern is
    /// fixed and the client regenerated, the pinned assistant thread only
    /// appears here once it has other activity in `threads`.
    func defaultThread() async -> ThreadID? {
        if let cached = try? await transcriptStore.threads(), let first = cached.first {
            return first.id
        }
        guard let session = ensureSession() else { return nil }
        let remote = (try? await session.threads()) ?? []
        return remote.first?.id
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
        approvalsStore?.stop()
        approvalsStore = nil
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
