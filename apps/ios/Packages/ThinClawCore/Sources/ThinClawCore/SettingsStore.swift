import Foundation
import Observation

/// The paired-gateway identity captured at pairing, read from the Keychain
/// credential. Abstracted so ``SettingsStore`` never links `ThinClawAuth`; the
/// `FeatureSettings` adapter reads it from `DeviceCredential`.
public struct PairedGatewayIdentity: Hashable, Sendable {
    public var gatewayName: String
    public var instanceID: String
    public var gatewayURL: String?
    public var pinnedFingerprint: String?

    public init(
        gatewayName: String,
        instanceID: String,
        gatewayURL: String?,
        pinnedFingerprint: String?
    ) {
        self.gatewayName = gatewayName
        self.instanceID = instanceID
        self.gatewayURL = gatewayURL
        self.pinnedFingerprint = pinnedFingerprint
    }
}

/// The unpair action, abstracted so the store can trigger self-revoke + local
/// erase + return-to-onboarding without linking the app composition root. The
/// production adapter calls `AppDependencies.unpair()`.
public protocol Unpairing: Sendable {
    func unpair() async
}

/// A closure-backed ``Unpairing`` so the App composition root can bridge onto
/// `AppDependencies.unpair()` without a bespoke adapter type.
public struct ClosureUnpairing: Unpairing {
    private let action: @Sendable () async -> Void

    public init(_ action: @escaping @Sendable () async -> Void) {
        self.action = action
    }

    public func unpair() async {
        await action()
    }
}

/// The enhanced-protection wiring (docs/MOBILE_SECURITY.md, "Data at rest"):
/// upgrades the transcript cache from
/// `NSFileProtectionCompleteUntilFirstUserAuthentication` to `Complete`
/// (documented cost: no locked-screen refresh) and **persists** the operator's
/// choice so the app-switcher redaction overlay and next-launch re-tag both read
/// it. Abstracted so the store owns neither `ThinClawPersistence`/GRDB nor the
/// shared-defaults key.
///
/// The production adapter (a) persists the Bool under the shared
/// `PrivacySettingsKey.enhancedProtection` defaults key the privacy overlay's
/// `@AppStorage` observes, and (b) re-tags the transcript store's files.
/// ``apply(enhanced:)`` returns whether the *file re-tag* took effect; a `false`
/// (e.g. in-memory cache, or the API is unavailable) does not undo the persisted
/// preference — it still applies on next launch.
public protocol TranscriptProtectionControlling: Sendable {
    /// The persisted enhanced-protection preference at construction time, so the
    /// store's initial toggle value matches what the overlay already reads.
    var current: Bool { get }

    /// Persist the choice and apply the file re-tag. Returns whether the re-tag
    /// took effect.
    @discardableResult
    func apply(enhanced: Bool) async -> Bool
}

/// A live source of the client connection state, abstracted so the store folds a
/// stream on macOS without linking the transport. The production adapter bridges
/// `GatewaySession.connectionState`.
public protocol ConnectionStateSource: Sendable {
    func connectionStates() -> AsyncStream<ConnectionState>
}

/// A closure-backed ``TranscriptProtectionControlling`` so the App composition
/// root can wire the concrete `GRDBTranscriptStore.applyFileProtection(enhanced:)`
/// without `ThinClawCore` (or `FeatureSettings`) depending on
/// `ThinClawPersistence`/GRDB. When the transcript store is the in-memory
/// fallback (no file to protect) the App passes a closure returning `false`.
public struct ClosureProtectionControl: TranscriptProtectionControlling {
    public let current: Bool
    private let action: @Sendable (Bool) async -> Bool

    public init(current: Bool, _ action: @escaping @Sendable (Bool) async -> Bool) {
        self.current = current
        self.action = action
    }

    public func apply(enhanced: Bool) async -> Bool {
        await action(enhanced)
    }
}

/// Drives the in-app Settings surface (docs/MOBILE_APP.md M5): this device's
/// identity + scopes, companion (watch) listing and revoke, unpair, per-category
/// notification preview preferences, the paired-gateway connection summary with
/// a biometric-gated URL/pin reveal (D-K3), and the enhanced-protection toggle.
///
/// UI-free by design: it imports no SwiftUI/design layer, so every flow (device
/// load, companion revoke, unpair, preference persistence, gated reveal) is
/// exercised by plain `swift test` on macOS with mocked seams. `FeatureSettings`
/// supplies the SwiftUI screen and the concrete adapters.
///
/// Omitted by contract: device self-rename and self-rotate — the gateway exposes
/// only admin `/{id}/rename` and `/{id}/rotate`, which reject a device token, so
/// there is nothing for the phone to call over its own credential.
@MainActor
@Observable
public final class SettingsStore {
    // MARK: - Device management

    /// This device's own record (`GET /api/devices/me`); `nil` until loaded.
    public private(set) var thisDevice: ManagedDevice?
    /// This device's companions — the paired watch(es).
    public private(set) var companions: [ManagedDevice] = []
    /// Set while a device-management network call is in flight.
    public private(set) var isLoadingDevices: Bool = false
    /// The last device-management error, surfaced to the UI; cleared on success.
    public private(set) var deviceError: String?

    // MARK: - Notification preferences

    /// Per-category preview preferences (D-N3), persisted to the App Group.
    public private(set) var notificationPreferences: NotificationPreferences

    // MARK: - Connection

    /// The paired-gateway identity + live reachability. The URL/pin detail stays
    /// `nil` until ``revealConnectionDetail()`` clears the biometric gate.
    public private(set) var connectionInfo: GatewayConnectionInfo

    // MARK: - Data at rest

    /// Whether "Enhanced protection" (transcript cache `Complete`) is on.
    public private(set) var enhancedProtection: Bool

    // MARK: - Seams

    private let devices: any DeviceManaging
    private let identity: PairedGatewayIdentity
    private let biometrics: any BiometricGating
    private let unpairer: any Unpairing
    private let preferencesStore: NotificationPreferencesStore
    private let protectionControl: any TranscriptProtectionControlling
    private let connectionSource: any ConnectionStateSource

    private var connectionTask: Task<Void, Never>?

    public init(
        devices: any DeviceManaging,
        identity: PairedGatewayIdentity,
        biometrics: any BiometricGating,
        unpairer: any Unpairing,
        keyValueStore: any KeyValueStoring,
        protectionControl: any TranscriptProtectionControlling,
        connectionSource: any ConnectionStateSource
    ) {
        self.devices = devices
        self.identity = identity
        self.biometrics = biometrics
        self.unpairer = unpairer
        self.preferencesStore = NotificationPreferencesStore(store: keyValueStore)
        self.protectionControl = protectionControl
        self.connectionSource = connectionSource
        self.notificationPreferences = NotificationPreferencesStore(store: keyValueStore).load()
        self.connectionInfo = GatewayConnectionInfo(
            gatewayName: identity.gatewayName,
            instanceID: identity.instanceID)
        self.enhancedProtection = protectionControl.current
    }

    // MARK: - Lifecycle

    /// Load this device + companions and begin folding connection-state changes.
    public func start() async {
        subscribeToConnectionState()
        await refreshDevices()
    }

    /// Stop folding connection-state changes.
    public func stop() {
        connectionTask?.cancel()
        connectionTask = nil
    }

    // MARK: - Device management

    /// (Re)load this device's record and its companions from the gateway.
    public func refreshDevices() async {
        isLoadingDevices = true
        deviceError = nil
        defer { isLoadingDevices = false }
        do {
            async let me = devices.thisDevice()
            async let comps = devices.companions()
            self.thisDevice = try await me
            self.companions = try await comps
        } catch {
            deviceError = Self.describe(error)
        }
    }

    /// Revoke a companion (the paired watch) by id
    /// (`DELETE /api/devices/me/companions/{id}`). On success the companion is
    /// removed from ``companions`` without a full reload; on failure the error is
    /// surfaced and the list is left intact.
    public func revokeCompanion(id: String) async {
        deviceError = nil
        do {
            try await devices.revokeCompanion(id: id)
            companions.removeAll { $0.id == id }
        } catch {
            deviceError = Self.describe(error)
        }
    }

    /// Sign this device out: self-revoke + erase credential + return to
    /// onboarding, via the injected ``Unpairing`` (`AppDependencies.unpair()`).
    public func unpair() async {
        await unpairer.unpair()
    }

    // MARK: - Notification preferences

    /// Set the preview mode for one category and persist the whole set to the
    /// App Group so the NSE reads it. Invalid mode/category pairs are coerced by
    /// the model (e.g. `appOnly` on a non-approval category → `never`).
    public func setPreviewMode(_ mode: PreviewMode, for category: NotificationCategory) {
        notificationPreferences = notificationPreferences.setting(mode, for: category)
        preferencesStore.save(notificationPreferences)
    }

    // MARK: - Connection

    private func subscribeToConnectionState() {
        guard connectionTask == nil else { return }
        connectionTask = Task { [weak self] in
            guard let stream = self?.connectionSource.connectionStates() else { return }
            for await state in stream {
                guard let self else { return }
                self.connectionInfo.reachability = GatewayReachability(state)
            }
        }
    }

    /// Reveal the gateway URL + pinned fingerprint behind a Face ID gate (D-K3):
    /// these identify the operator's gateway, so they are surfaced only after a
    /// successful biometric prompt. Returns whether the reveal succeeded; on
    /// failure/cancel ``connectionInfo`` keeps `revealedDetail == nil`.
    @discardableResult
    public func revealConnectionDetail() async -> Bool {
        let ok = await biometrics.authenticate(
            reason: "Reveal your gateway connection details")
        guard ok else { return false }
        connectionInfo.revealedDetail = .init(
            gatewayURL: identity.gatewayURL ?? "—",
            pinnedFingerprint: identity.pinnedFingerprint)
        return true
    }

    /// Hide a previously-revealed connection detail (e.g. on backgrounding).
    public func hideConnectionDetail() {
        connectionInfo.revealedDetail = nil
    }

    // MARK: - Data at rest

    /// Toggle "Enhanced protection". The protection control persists the choice
    /// (to the shared defaults key the app-switcher overlay reads) and applies
    /// the transcript-cache re-tag. A failed live re-tag does not undo the
    /// persisted preference — it applies on next launch.
    public func setEnhancedProtection(_ enabled: Bool) async {
        enhancedProtection = enabled
        _ = await protectionControl.apply(enhanced: enabled)
    }

    // MARK: - Helpers

    private static func describe(_ error: Error) -> String {
        let text = (error as? LocalizedError)?.errorDescription ?? "\(error)"
        return text.isEmpty ? "Something went wrong." : text
    }
}
