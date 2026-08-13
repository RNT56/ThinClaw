#if canImport(CryptoKit) && canImport(Security)
    import CryptoKit
    import Foundation
    import ThinClawAuth

    /// Per-pairing key material used only to authenticate phone→Watch control
    /// commands. It is independent of both bearer tokens: the Watch never
    /// learns the phone token, and a control message never carries the Watch
    /// token.
    public struct WatchControlMaterial: Codable, Sendable, Equatable {
        public var generation: UInt64
        public var installationID: String
        public var parentDeviceID: String
        public var authenticationKey: Data

        public init(
            generation: UInt64,
            installationID: String,
            parentDeviceID: String,
            authenticationKey: Data
        ) {
            self.generation = generation
            self.installationID = installationID
            self.parentDeviceID = parentDeviceID
            self.authenticationKey = authenticationKey
        }
    }

    /// Authenticated, token-free command asking a Watch to erase all state for
    /// one provisioning generation.
    public struct WatchWipeCommand: Codable, Sendable, Equatable {
        public var version: Int
        public var commandID: String
        public var generation: UInt64
        public var parentDeviceID: String
        public var authenticationTag: String

        public static let currentVersion = 1
        public static let contextKey = "watchWipeCommand"
        public static let deprovisionedKey = "watchDeprovisioned"

        public init(
            version: Int = WatchWipeCommand.currentVersion,
            commandID: String,
            generation: UInt64,
            parentDeviceID: String,
            authenticationTag: String
        ) {
            self.version = version
            self.commandID = commandID
            self.generation = generation
            self.parentDeviceID = parentDeviceID
            self.authenticationTag = authenticationTag
        }

        public static func authenticated(
            material: WatchControlMaterial,
            commandID: String = UUID().uuidString
        ) -> WatchWipeCommand {
            var command = WatchWipeCommand(
                commandID: commandID,
                generation: material.generation,
                parentDeviceID: material.parentDeviceID,
                authenticationTag: "")
            command.authenticationTag = WatchControlAuthentication.tag(
                for: command.signingPayload,
                key: material.authenticationKey)
            return command
        }

        public func isAuthenticated(by material: WatchControlMaterial) -> Bool {
            version == Self.currentVersion
                && generation == material.generation
                && parentDeviceID == material.parentDeviceID
                && WatchControlAuthentication.verify(
                    authenticationTag,
                    payload: signingPayload,
                    key: material.authenticationKey)
        }

        public func applicationContext() throws -> [String: Any] {
            [
                Self.contextKey: try JSONEncoder().encode(self),
                Self.deprovisionedKey: true,
            ]
        }

        public func messagePayload() throws -> [String: Any] {
            [Self.contextKey: try JSONEncoder().encode(self)]
        }

        public static func fromPayload(_ payload: [String: Any]) throws -> WatchWipeCommand? {
            guard let data = payload[contextKey] as? Data else { return nil }
            let command = try JSONDecoder().decode(WatchWipeCommand.self, from: data)
            guard command.version == currentVersion else {
                throw WatchRelayError.unsupportedVersion(command.version)
            }
            return command
        }

        fileprivate var signingPayload: Data {
            WatchControlAuthentication.canonical([
                String(version), commandID, String(generation), parentDeviceID,
            ])
        }
    }

    /// Watch→phone acknowledgement. It is authenticated with the same
    /// generation key, so an unrelated/stale WatchConnectivity payload cannot
    /// make the phone forget a durable pending wipe.
    public struct WatchWipeAcknowledgement: Codable, Sendable, Equatable {
        public var version: Int
        public var commandID: String
        public var generation: UInt64
        public var authenticationTag: String

        public static let currentVersion = 1
        public static let contextKey = "watchWipeAcknowledgement"

        public init(
            version: Int = WatchWipeAcknowledgement.currentVersion,
            commandID: String,
            generation: UInt64,
            authenticationTag: String
        ) {
            self.version = version
            self.commandID = commandID
            self.generation = generation
            self.authenticationTag = authenticationTag
        }

        public static func authenticated(
            command: WatchWipeCommand,
            material: WatchControlMaterial
        ) -> WatchWipeAcknowledgement {
            var acknowledgement = WatchWipeAcknowledgement(
                commandID: command.commandID,
                generation: command.generation,
                authenticationTag: "")
            acknowledgement.authenticationTag = WatchControlAuthentication.tag(
                for: acknowledgement.signingPayload,
                key: material.authenticationKey)
            return acknowledgement
        }

        public func isAuthenticated(
            for command: WatchWipeCommand,
            material: WatchControlMaterial
        ) -> Bool {
            version == Self.currentVersion
                && commandID == command.commandID
                && generation == command.generation
                && WatchControlAuthentication.verify(
                    authenticationTag,
                    payload: signingPayload,
                    key: material.authenticationKey)
        }

        public func messagePayload() throws -> [String: Any] {
            [Self.contextKey: try JSONEncoder().encode(self)]
        }

        public static func fromPayload(
            _ payload: [String: Any]
        ) throws -> WatchWipeAcknowledgement? {
            guard let data = payload[contextKey] as? Data else { return nil }
            let acknowledgement = try JSONDecoder().decode(
                WatchWipeAcknowledgement.self, from: data)
            guard acknowledgement.version == currentVersion else {
                throw WatchRelayError.unsupportedVersion(acknowledgement.version)
            }
            return acknowledgement
        }

        fileprivate var signingPayload: Data {
            WatchControlAuthentication.canonical([
                String(version), commandID, String(generation), "acknowledged",
            ])
        }
    }

    /// Non-secret durable replay marker kept after the Watch credential and its
    /// control key are erased. The stored signed acknowledgement lets an exact
    /// duplicate retry be acknowledged without retaining any secret.
    public struct WatchWipeTombstone: Codable, Sendable, Equatable {
        public var generation: UInt64
        public var parentDeviceID: String
        public var commandID: String
        public var commandAuthenticationTag: String
        public var acknowledgement: WatchWipeAcknowledgement

        public init(
            generation: UInt64,
            parentDeviceID: String,
            commandID: String,
            commandAuthenticationTag: String,
            acknowledgement: WatchWipeAcknowledgement
        ) {
            self.generation = generation
            self.parentDeviceID = parentDeviceID
            self.commandID = commandID
            self.commandAuthenticationTag = commandAuthenticationTag
            self.acknowledgement = acknowledgement
        }

        public func matches(_ command: WatchWipeCommand) -> Bool {
            generation == command.generation
                && parentDeviceID == command.parentDeviceID
                && commandID == command.commandID
                && commandAuthenticationTag == command.authenticationTag
        }
    }

    public enum WatchWipeDecision: Sendable, Equatable {
        /// Persist the tombstone first, then erase credential/mirrors and send
        /// the acknowledgement.
        case apply(WatchWipeTombstone)
        /// The exact command was already applied; resend its durable ack.
        case acknowledge(WatchWipeAcknowledgement)
        /// Unauthenticated, stale, or unrelated command.
        case reject
    }

    /// Pure Watch-side authentication/idempotence policy.
    public enum WatchWipeReceiver {
        public static func evaluate(
            _ command: WatchWipeCommand,
            credentialMaterial: WatchControlMaterial?,
            tombstone: WatchWipeTombstone?
        ) -> WatchWipeDecision {
            if let tombstone, tombstone.matches(command) {
                return .acknowledge(tombstone.acknowledgement)
            }
            if let tombstone, command.generation <= tombstone.generation {
                return .reject
            }
            guard let material = credentialMaterial,
                command.isAuthenticated(by: material)
            else { return .reject }

            let acknowledgement = WatchWipeAcknowledgement.authenticated(
                command: command, material: material)
            return .apply(
                WatchWipeTombstone(
                    generation: command.generation,
                    parentDeviceID: command.parentDeviceID,
                    commandID: command.commandID,
                    commandAuthenticationTag: command.authenticationTag,
                    acknowledgement: acknowledgement))
        }

        /// Accept the current generation again (the phone may re-mint its
        /// companion token), accept a genuinely newer generation, and reject
        /// provisioning that a tombstone or newer credential proves stale.
        public static func acceptsProvisioning(
            generation: UInt64,
            currentGeneration: UInt64?,
            tombstone: WatchWipeTombstone?
        ) -> Bool {
            if let tombstone, generation <= tombstone.generation { return false }
            if let currentGeneration, generation < currentGeneration { return false }
            return true
        }
    }

    /// Phone-side persisted command plus the material needed to validate its
    /// eventual acknowledgement. Multiple generations may be pending during a
    /// rapid gateway replacement; the Watch's tombstone ordering makes their
    /// later delivery safe.
    public struct PhoneWatchWipePlan: Codable, Sendable, Equatable {
        public var command: WatchWipeCommand
        public var material: WatchControlMaterial
        public var companionDeviceID: String?

        public init(
            command: WatchWipeCommand,
            material: WatchControlMaterial,
            companionDeviceID: String?
        ) {
            self.command = command
            self.material = material
            self.companionDeviceID = companionDeviceID
        }
    }

    /// Crash-safe phone-side generation/outbox store. The HMAC key is kept in
    /// the phone Keychain; the exact command remains pending across launches and
    /// offline Watch periods until an authenticated ack arrives.
    public final class PhoneWatchControlStore: @unchecked Sendable {
        private struct State: Codable {
            var version = 1
            var lastGeneration: UInt64 = 0
            var activeMaterial: WatchControlMaterial?
            var activeCompanionDeviceID: String?
            var pendingWipes: [PhoneWatchWipePlan] = []
        }

        public static let keychainKey = "phone-watch-control-state-v1"

        private let keychain: any KeychainStoring
        private let now: @Sendable () -> Date
        private let randomKey: @Sendable () -> Data
        private let lock = NSLock()

        public init(
            keychain: any KeychainStoring,
            now: @escaping @Sendable () -> Date = { .now },
            randomKey: @escaping @Sendable () -> Data = {
                Data(SymmetricKey(size: .bits256).withUnsafeBytes { Array($0) })
            }
        ) {
            self.keychain = keychain
            self.now = now
            self.randomKey = randomKey
        }

        public func prepare(for parent: DeviceCredential) throws -> WatchControlMaterial {
            try lock.withLock {
                guard let parentDeviceID = parent.deviceID, !parentDeviceID.isEmpty else {
                    throw PhoneWatchControlError.missingParentDeviceID
                }
                var state = try load()
                if let active = state.activeMaterial,
                    active.installationID == parent.installationID,
                    active.parentDeviceID == parentDeviceID,
                    !state.pendingWipes.contains(where: {
                        $0.material.generation == active.generation
                    })
                {
                    return active
                }

                let wallClockGeneration = UInt64(
                    max(0, now().timeIntervalSince1970 * 1_000))
                let generation = max(state.lastGeneration &+ 1, wallClockGeneration)
                let material = WatchControlMaterial(
                    generation: generation,
                    installationID: parent.installationID,
                    parentDeviceID: parentDeviceID,
                    authenticationKey: randomKey())
                state.lastGeneration = generation
                state.activeMaterial = material
                state.activeCompanionDeviceID = nil
                try save(state)
                return material
            }
        }

        public func activeMaterial(for parent: DeviceCredential) throws -> WatchControlMaterial? {
            try lock.withLock {
                guard let parentDeviceID = parent.deviceID else { return nil }
                let state = try load()
                guard state.activeMaterial?.installationID == parent.installationID,
                    state.activeMaterial?.parentDeviceID == parentDeviceID
                else { return nil }
                return state.activeMaterial
            }
        }

        public func activeCompanionDeviceID(generation: UInt64) throws -> String? {
            try lock.withLock {
                let state = try load()
                guard state.activeMaterial?.generation == generation else { return nil }
                return state.activeCompanionDeviceID
            }
        }

        public func recordCompanion(deviceID: String, generation: UInt64) throws {
            try lock.withLock {
                var state = try load()
                guard state.activeMaterial?.generation == generation else { return }
                state.activeCompanionDeviceID = deviceID
                try save(state)
            }
        }

        /// Persist (or return) the exact idempotent command before any transport
        /// attempt. Calling this repeatedly for one generation never creates a
        /// second command.
        public func beginWipe(for parent: DeviceCredential) throws -> PhoneWatchWipePlan {
            try lock.withLock {
                guard let parentDeviceID = parent.deviceID, !parentDeviceID.isEmpty else {
                    throw PhoneWatchControlError.missingParentDeviceID
                }
                var state = try load()
                guard let material = state.activeMaterial,
                    material.installationID == parent.installationID,
                    material.parentDeviceID == parentDeviceID
                else {
                    // Retrying after the active material was retired must return
                    // the exact durable command, not manufacture a new one.
                    if let existing = state.pendingWipes.last(where: {
                        $0.material.installationID == parent.installationID
                            && $0.material.parentDeviceID == parentDeviceID
                    }) {
                        return existing
                    }
                    throw PhoneWatchControlError.noActiveGeneration
                }
                if let existing = state.pendingWipes.first(where: {
                    $0.material.generation == material.generation
                }) {
                    return existing
                }

                let plan = PhoneWatchWipePlan(
                    command: .authenticated(material: material),
                    material: material,
                    companionDeviceID: state.activeCompanionDeviceID)
                state.pendingWipes.append(plan)
                // A wiped generation must never be reused after its ack removes
                // the outbox entry. Retire it atomically with the durable command;
                // a later pairing prepares genuinely newer control material.
                state.activeMaterial = nil
                state.activeCompanionDeviceID = nil
                try save(state)
                return plan
            }
        }

        public func pendingWipes() throws -> [PhoneWatchWipePlan] {
            try lock.withLock { try load().pendingWipes }
        }

        /// Remove a pending command only after its matching HMAC-authenticated
        /// acknowledgement has been verified.
        @discardableResult
        public func acknowledge(_ acknowledgement: WatchWipeAcknowledgement) throws -> Bool {
            try lock.withLock {
                var state = try load()
                guard
                    let index = state.pendingWipes.firstIndex(where: {
                        acknowledgement.isAuthenticated(
                            for: $0.command, material: $0.material)
                    })
                else { return false }
                state.pendingWipes.remove(at: index)
                try save(state)
                return true
            }
        }

        private func load() throws -> State {
            guard let data = try keychain.secret(for: Self.keychainKey) else {
                return State()
            }
            return try JSONDecoder().decode(State.self, from: data)
        }

        private func save(_ state: State) throws {
            try keychain.setSecret(
                JSONEncoder().encode(state),
                for: Self.keychainKey,
                accessibility: .afterFirstUnlockDeviceOnly)
        }
    }

    public enum PhoneWatchControlError: Error, Sendable, Equatable {
        case noActiveGeneration
        case missingParentDeviceID
    }

    private enum WatchControlAuthentication {
        static func canonical(_ fields: [String]) -> Data {
            // Length-prefix every field so concatenation cannot be ambiguous.
            Data(fields.map { "\($0.utf8.count):\($0)" }.joined(separator: "|").utf8)
        }

        static func tag(for payload: Data, key: Data) -> String {
            let code = HMAC<SHA256>.authenticationCode(
                for: payload, using: SymmetricKey(data: key))
            return Data(code).base64URLEncodedString()
        }

        static func verify(_ encodedTag: String, payload: Data, key: Data) -> Bool {
            guard let tag = Data(base64URLEncoded: encodedTag) else { return false }
            return HMAC<SHA256>.isValidAuthenticationCode(
                tag, authenticating: payload, using: SymmetricKey(data: key))
        }
    }

    extension Data {
        fileprivate func base64URLEncodedString() -> String {
            base64EncodedString()
                .replacingOccurrences(of: "+", with: "-")
                .replacingOccurrences(of: "/", with: "_")
                .replacingOccurrences(of: "=", with: "")
        }

        fileprivate init?(base64URLEncoded value: String) {
            var base64 =
                value
                .replacingOccurrences(of: "-", with: "+")
                .replacingOccurrences(of: "_", with: "/")
            let padding = (4 - base64.count % 4) % 4
            base64.append(String(repeating: "=", count: padding))
            self.init(base64Encoded: base64)
        }
    }
#endif
