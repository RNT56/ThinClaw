import Foundation
import Testing

@testable import ThinClawCore

// MARK: - Test doubles

/// A scripted ``DeviceManaging`` that serves canned records and records revoke
/// calls, so the settings store's device flows run without a live gateway.
private final class MockDeviceManager: DeviceManaging, @unchecked Sendable {
    private let lock = NSLock()
    private var _me: ManagedDevice
    private var _companions: [ManagedDevice]
    private var _revoked: [String] = []
    private var _failThisDevice = false
    private var _failRevoke = false

    init(me: ManagedDevice, companions: [ManagedDevice]) {
        self._me = me
        self._companions = companions
    }

    var revoked: [String] { lock.withLock { _revoked } }

    func failThisDevice() { lock.withLock { _failThisDevice = true } }
    func failRevoke() { lock.withLock { _failRevoke = true } }

    func thisDevice() async throws -> ManagedDevice {
        let fail = lock.withLock { _failThisDevice }
        if fail {
            struct LoadFailed: LocalizedError { var errorDescription: String? { "device load failed" } }
            throw LoadFailed()
        }
        return lock.withLock { _me }
    }

    func companions() async throws -> [ManagedDevice] {
        lock.withLock { _companions }
    }

    func revokeCompanion(id: String) async throws {
        let fail = lock.withLock { _failRevoke }
        if fail {
            struct RevokeFailed: Error {}
            throw RevokeFailed()
        }
        lock.withLock { _revoked.append(id) }
    }
}

/// A biometric gate with a controllable result and an invocation counter.
private final class MockBiometrics: BiometricGating, @unchecked Sendable {
    private let lock = NSLock()
    private var _result: Bool
    private var _invocations = 0

    init(result: Bool) { self._result = result }

    var invocations: Int { lock.withLock { _invocations } }
    func setResult(_ value: Bool) { lock.withLock { _result = value } }

    func authenticate(reason: String) async -> Bool {
        lock.withLock {
            _invocations += 1
            return _result
        }
    }
}

/// Records whether ``unpair`` was called.
private final class MockUnpairer: Unpairing, @unchecked Sendable {
    private let lock = NSLock()
    private var _count = 0
    var count: Int { lock.withLock { _count } }
    func unpair() async { lock.withLock { _count += 1 } }
}

/// Records the last requested protection level, persists it like the production
/// adapter, and reports a scripted re-tag result.
private final class MockProtectionControl: TranscriptProtectionControlling, @unchecked Sendable {
    private let lock = NSLock()
    private var _applied: [Bool] = []
    private var _result: Bool
    private var _persisted: Bool

    init(result: Bool = true, initial: Bool = false) {
        self._result = result
        self._persisted = initial
    }

    var applied: [Bool] { lock.withLock { _applied } }
    /// The persisted preference (what survives to the next launch / the overlay).
    var persisted: Bool { lock.withLock { _persisted } }

    var current: Bool { lock.withLock { _persisted } }

    func apply(enhanced: Bool) async -> Bool {
        lock.withLock {
            _applied.append(enhanced)
            // The production adapter persists regardless of the re-tag result.
            _persisted = enhanced
            return _result
        }
    }
}

/// A connection-state source the test can drive.
private final class MockConnectionSource: ConnectionStateSource, @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: AsyncStream<ConnectionState>.Continuation?

    func connectionStates() -> AsyncStream<ConnectionState> {
        AsyncStream { continuation in
            self.lock.withLock { self.continuation = continuation }
        }
    }

    func emit(_ state: ConnectionState) {
        lock.withLock { continuation }?.yield(state)
    }
}

// MARK: - Fixtures

private func phone() -> ManagedDevice {
    ManagedDevice(
        id: "dev-phone",
        name: "My iPhone",
        platform: "ios",
        scopes: [.chat, .approvals, .jobsRead, .devicesSelf],
        lastSeenAt: "2026-07-05T12:00:00Z",
        tokenPrefix: "tcd_ab",
        parentDeviceID: nil)
}

private func watch() -> ManagedDevice {
    ManagedDevice(
        id: "dev-watch",
        name: "My Watch",
        platform: "watchos",
        scopes: [.chat, .approvals],
        lastSeenAt: "2026-07-05T11:00:00Z",
        tokenPrefix: "tcd_cd",
        parentDeviceID: "dev-phone")
}

private func identity() -> PairedGatewayIdentity {
    PairedGatewayIdentity(
        gatewayName: "Home Gateway",
        instanceID: "inst-123",
        gatewayURL: "https://gw.example.ts.net",
        pinnedFingerprint: "AA:BB:CC")
}

@MainActor
private func makeStore(
    devices: MockDeviceManager,
    biometrics: MockBiometrics = MockBiometrics(result: true),
    unpairer: MockUnpairer = MockUnpairer(),
    kv: InMemoryKeyValueStore = InMemoryKeyValueStore(),
    protection: MockProtectionControl = MockProtectionControl(),
    connection: MockConnectionSource = MockConnectionSource()
) -> SettingsStore {
    SettingsStore(
        devices: devices,
        identity: identity(),
        biometrics: biometrics,
        unpairer: unpairer,
        keyValueStore: kv,
        protectionControl: protection,
        connectionSource: connection)
}

// MARK: - Device loading

@MainActor
@Test func refreshLoadsThisDeviceAndCompanions() async {
    let devices = MockDeviceManager(me: phone(), companions: [watch()])
    let store = makeStore(devices: devices)

    await store.refreshDevices()

    #expect(store.thisDevice?.id == "dev-phone")
    #expect(store.thisDevice?.scopes.contains(.devicesSelf) == true)
    #expect(store.companions.map(\.id) == ["dev-watch"])
    #expect(store.companions.first?.isCompanion == true)
    #expect(store.deviceError == nil)
    #expect(store.isLoadingDevices == false)
}

@MainActor
@Test func refreshSurfacesErrorOnFailure() async {
    let devices = MockDeviceManager(me: phone(), companions: [])
    devices.failThisDevice()
    let store = makeStore(devices: devices)

    await store.refreshDevices()

    #expect(store.thisDevice == nil)
    #expect(store.deviceError == "device load failed")
    #expect(store.isLoadingDevices == false)
}

// MARK: - Companion revoke

@MainActor
@Test func revokeCompanionRemovesItLocally() async {
    let devices = MockDeviceManager(me: phone(), companions: [watch()])
    let store = makeStore(devices: devices)
    await store.refreshDevices()

    await store.revokeCompanion(id: "dev-watch")

    #expect(devices.revoked == ["dev-watch"])
    #expect(store.companions.isEmpty)
    #expect(store.deviceError == nil)
}

@MainActor
@Test func revokeFailureKeepsCompanionAndSurfacesError() async {
    let devices = MockDeviceManager(me: phone(), companions: [watch()])
    devices.failRevoke()
    let store = makeStore(devices: devices)
    await store.refreshDevices()

    await store.revokeCompanion(id: "dev-watch")

    #expect(devices.revoked.isEmpty)
    #expect(store.companions.map(\.id) == ["dev-watch"])
    #expect(store.deviceError != nil)
}

// MARK: - Unpair

@MainActor
@Test func unpairDelegatesToUnpairer() async {
    let devices = MockDeviceManager(me: phone(), companions: [])
    let unpairer = MockUnpairer()
    let store = makeStore(devices: devices, unpairer: unpairer)

    await store.unpair()

    #expect(unpairer.count == 1)
}

// MARK: - Biometric-gated connection reveal (D-K3)

@MainActor
@Test func revealRequiresBiometricSuccess() async {
    let devices = MockDeviceManager(me: phone(), companions: [])
    let biometrics = MockBiometrics(result: false)
    let store = makeStore(devices: devices, biometrics: biometrics)

    let ok = await store.revealConnectionDetail()

    #expect(ok == false)
    #expect(biometrics.invocations == 1)
    #expect(store.connectionInfo.revealedDetail == nil)
}

@MainActor
@Test func revealSurfacesUrlAndPinAfterBiometricSuccess() async {
    let devices = MockDeviceManager(me: phone(), companions: [])
    let biometrics = MockBiometrics(result: true)
    let store = makeStore(devices: devices, biometrics: biometrics)

    let ok = await store.revealConnectionDetail()

    #expect(ok == true)
    #expect(store.connectionInfo.revealedDetail?.gatewayURL == "https://gw.example.ts.net")
    #expect(store.connectionInfo.revealedDetail?.pinnedFingerprint == "AA:BB:CC")

    store.hideConnectionDetail()
    #expect(store.connectionInfo.revealedDetail == nil)
}

// MARK: - Connection state folding

@MainActor
@Test func connectionStateFoldsIntoReachability() async {
    let devices = MockDeviceManager(me: phone(), companions: [])
    let connection = MockConnectionSource()
    let store = makeStore(devices: devices, connection: connection)

    #expect(store.connectionInfo.gatewayName == "Home Gateway")
    #expect(store.connectionInfo.instanceID == "inst-123")
    #expect(store.connectionInfo.reachability == .offline)

    await store.start()
    connection.emit(.connected)
    // Allow the folding task to observe the emission.
    try? await Task.sleep(nanoseconds: 50_000_000)
    #expect(store.connectionInfo.reachability == .reachable)

    connection.emit(.reconnecting(attempt: 1))
    try? await Task.sleep(nanoseconds: 50_000_000)
    #expect(store.connectionInfo.reachability == .degraded)

    store.stop()
}

// MARK: - Enhanced protection

@MainActor
@Test func enhancedProtectionPersistsAndApplies() async {
    let devices = MockDeviceManager(me: phone(), companions: [])
    let protection = MockProtectionControl(result: true)
    let store = makeStore(devices: devices, protection: protection)

    #expect(store.enhancedProtection == false)

    await store.setEnhancedProtection(true)

    #expect(store.enhancedProtection == true)
    #expect(protection.applied == [true])
    #expect(protection.persisted == true)

    // A fresh store over a control that already persisted `true` reads it back
    // as the initial toggle value (matches the app-switcher overlay's read).
    let store2 = makeStore(
        devices: devices, protection: MockProtectionControl(initial: true))
    #expect(store2.enhancedProtection == true)
}

@MainActor
@Test func enhancedProtectionPreferenceSticksEvenWhenApplyFails() async {
    let devices = MockDeviceManager(me: phone(), companions: [])
    let protection = MockProtectionControl(result: false)
    let store = makeStore(devices: devices, protection: protection)

    await store.setEnhancedProtection(true)

    // The live re-tag failed, but the preference is persisted so it applies on
    // next launch.
    #expect(store.enhancedProtection == true)
    #expect(protection.persisted == true)
}
