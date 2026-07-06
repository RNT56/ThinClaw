import Foundation
import ThinClawAPI
import ThinClawAuth
import ThinClawCore
import ThinClawTransport

#if canImport(LocalAuthentication)
    import LocalAuthentication
#endif

// MARK: - Device management

/// Production ``DeviceManaging`` over the generated `ThinClawAPI` client
/// (`GET /api/devices/me`, `GET /api/devices/me/companions`,
/// `DELETE /api/devices/me/companions/{id}`), scoped to the current device's
/// token (`devices:self`). The store stays behind ``DeviceManaging`` so its
/// self/companion/revoke flows unit-test on macOS without a live gateway.
///
/// The client is the same pinned-session `Client` the app builds in
/// `AppDependencies.makePushClient()`, so device management rides the D-X2
/// pinned connection like every other REST call.
public struct GatewayDeviceManager: DeviceManaging {
    private let client: any APIProtocol

    public init(client: any APIProtocol) {
        self.client = client
    }

    public func thisDevice() async throws -> ManagedDevice {
        let output = try await client.devicesMeHandler(.init())
        let info = try output.ok.body.json
        return Self.map(info)
    }

    public func companions() async throws -> [ManagedDevice] {
        let output = try await client.devicesMeCompanionsListHandler(.init())
        let payload = try output.ok.body.json
        return payload.companions.map(Self.map)
    }

    public func revokeCompanion(id: String) async throws {
        let output = try await client.devicesMeCompanionsRevokeHandler(
            path: .init(id: id))
        // A 200 with the revoked record is success; any other case throws below.
        _ = try output.ok
    }

    /// Map a generated `DeviceInfo` into the UI-free ``ManagedDevice``.
    static func map(_ info: Components.Schemas.DeviceInfo) -> ManagedDevice {
        ManagedDevice(
            id: info.deviceId,
            name: info.name,
            platform: platformString(info.platform),
            scopes: info.scopes.map { DeviceScopeTag(wire: $0.rawValue) },
            lastSeenAt: info.lastSeenAt,
            tokenPrefix: info.tokenPrefix,
            parentDeviceID: info.parentDeviceId)
    }

    /// Flatten the generated `DevicePlatform` oneOf into a plain string.
    static func platformString(_ platform: Components.Schemas.DevicePlatform) -> String {
        switch platform {
        case .case1(let value): return value.rawValue
        case .case2(let value): return value.rawValue
        case .case3(let value): return value.rawValue
        case .case4(let value): return value.rawValue
        case .case5(let value): return value.other
        }
    }
}

// MARK: - Connection state

/// Bridges ``GatewaySession/connectionState`` (an actor-isolated
/// `AsyncStream`) into the store's ``ConnectionStateSource`` seam.
public struct GatewaySessionConnectionSource: ConnectionStateSource {
    private let session: GatewaySession

    public init(session: GatewaySession) {
        self.session = session
    }

    public func connectionStates() -> AsyncStream<ConnectionState> {
        AsyncStream { continuation in
            let task = Task {
                for await state in await session.connectionState {
                    continuation.yield(state)
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

// MARK: - App Group defaults

/// Production ``KeyValueStoring`` over the shared App Group `UserDefaults`
/// suite, so the settings UI and the Notification Service Extension read/write
/// the same notification-preview + enhanced-protection keys (D-K2/D-N3).
///
/// Falls back to writing nothing readable when the suite is unavailable (a bare
/// test host without the App Group entitlement) — callers treat a missing value
/// as the category default, so a `nil` suite degrades to defaults rather than
/// crashing.
public struct AppGroupDefaultsStore: KeyValueStoring {
    /// The shared App Group container id (D-K2), mirrored from
    /// `ThinClawWidgetKitShared.WidgetSnapshotAccess.appGroupID`. Kept as a
    /// literal here so the settings feature does not pull the widget graph in
    /// just for a suite name; the NSE reads the same suite.
    public static let appGroupID = "group.com.thinclaw.shared"

    // `UserDefaults` is documented thread-safe but is not `Sendable`-annotated
    // in the SDK; the suite is only read/written by key here, so the reference
    // is safe to share across isolation domains.
    private nonisolated(unsafe) let defaults: UserDefaults?

    public init(suiteName: String = AppGroupDefaultsStore.appGroupID) {
        self.defaults = UserDefaults(suiteName: suiteName)
    }

    public func string(forKey key: String) -> String? {
        defaults?.string(forKey: key)
    }

    public func set(_ value: String?, forKey key: String) {
        defaults?.set(value, forKey: key)
    }
}

// MARK: - Biometric gate (D-K3)

/// Production ``BiometricGating`` over `LocalAuthentication` for the settings
/// surface's Face-ID-gated connection-detail reveal (D-K3). A fresh biometrics
/// evaluation with no passcode fallback; any failure/cancel/unavailable state
/// resolves to `false` so the URL/pin stays hidden.
public struct SettingsBiometricGate: BiometricGating {
    public init() {}

    public func authenticate(reason: String) async -> Bool {
        #if canImport(LocalAuthentication)
            let context = LAContext()
            context.localizedCancelTitle = "Cancel"
            var error: NSError?
            guard
                context.canEvaluatePolicy(
                    .deviceOwnerAuthenticationWithBiometrics, error: &error)
            else {
                return false
            }
            return await withCheckedContinuation { continuation in
                context.evaluatePolicy(
                    .deviceOwnerAuthenticationWithBiometrics,
                    localizedReason: reason
                ) { success, _ in
                    continuation.resume(returning: success)
                }
            }
        #else
            return false
        #endif
    }
}

// MARK: - Identity

extension PairedGatewayIdentity {
    /// Build the settings identity from the stored ``DeviceCredential`` — the
    /// gateway name, instance id (`installationID`), the policy-allowed base URL,
    /// and the pinned TLS fingerprint captured at pairing.
    public init(credential: DeviceCredential) {
        self.init(
            gatewayName: credential.gatewayName,
            instanceID: credential.installationID,
            gatewayURL: credential.preferredBaseURL?.absoluteString,
            pinnedFingerprint: credential.serverFingerprint)
    }
}
