#if canImport(CryptoKit) && canImport(Security)
    import Foundation
    import Testing
    import ThinClawAuth

    @testable import ThinClawWatchBridge

    @Suite("Authenticated Watch deprovision control")
    struct WatchDeprovisionControlTests {
        private let material = WatchControlMaterial(
            generation: 41,
            installationID: "install-1",
            parentDeviceID: "phone-1",
            authenticationKey: Data(repeating: 0xA5, count: 32))

        @Test("Valid wipe applies, authenticates its ack, and duplicate is idempotent")
        func applyAndDuplicateAck() {
            let command = WatchWipeCommand.authenticated(
                material: material, commandID: "wipe-1")

            guard
                case .apply(let tombstone) = WatchWipeReceiver.evaluate(
                    command, credentialMaterial: material, tombstone: nil)
            else {
                Issue.record("expected authenticated wipe to apply")
                return
            }
            #expect(
                tombstone.acknowledgement.isAuthenticated(
                    for: command, material: material))

            #expect(
                WatchWipeReceiver.evaluate(
                    command, credentialMaterial: nil, tombstone: tombstone)
                    == .acknowledge(tombstone.acknowledgement))
        }

        @Test("Forged and stale commands fail closed")
        func forgedAndStaleReject() {
            var forged = WatchWipeCommand.authenticated(
                material: material, commandID: "wipe-forged")
            forged.parentDeviceID = "attacker"
            #expect(
                WatchWipeReceiver.evaluate(
                    forged, credentialMaterial: material, tombstone: nil) == .reject)

            let applied = WatchWipeCommand.authenticated(
                material: material, commandID: "wipe-applied")
            guard
                case .apply(let tombstone) = WatchWipeReceiver.evaluate(
                    applied, credentialMaterial: material, tombstone: nil)
            else { return }
            let staleMaterial = WatchControlMaterial(
                generation: 40,
                installationID: "install-1",
                parentDeviceID: "phone-1",
                authenticationKey: Data(repeating: 0xB4, count: 32))
            let stale = WatchWipeCommand.authenticated(
                material: staleMaterial, commandID: "wipe-stale")
            #expect(
                WatchWipeReceiver.evaluate(
                    stale, credentialMaterial: staleMaterial, tombstone: tombstone) == .reject)
        }

        @Test("Tombstone rejects old provisioning but permits a genuinely newer generation")
        func provisioningGenerationPolicy() {
            let command = WatchWipeCommand.authenticated(
                material: material, commandID: "wipe-1")
            guard
                case .apply(let tombstone) = WatchWipeReceiver.evaluate(
                    command, credentialMaterial: material, tombstone: nil)
            else { return }

            #expect(
                !WatchWipeReceiver.acceptsProvisioning(
                    generation: 41, currentGeneration: nil, tombstone: tombstone))
            #expect(
                !WatchWipeReceiver.acceptsProvisioning(
                    generation: 40, currentGeneration: nil, tombstone: tombstone))
            #expect(
                WatchWipeReceiver.acceptsProvisioning(
                    generation: 42, currentGeneration: nil, tombstone: tombstone))
        }
    }

    @Suite("Durable phone Watch wipe outbox")
    struct PhoneWatchControlStoreTests {
        private func credential() -> DeviceCredential {
            DeviceCredential(
                installationID: "install-1",
                deviceID: "phone-1",
                deviceToken: "tcd_phone",
                gatewayURLs: [URL(string: "https://gateway.example")!],
                serverFingerprint: "fp",
                gatewayName: "Gateway",
                pairedAt: Date(timeIntervalSince1970: 0))
        }

        @Test("Offline wipe survives reload and repeated begin is the same command")
        func offlineRetryIsDurableAndIdempotent() throws {
            let keychain = InMemoryKeychain()
            let store = PhoneWatchControlStore(
                keychain: keychain,
                now: { Date(timeIntervalSince1970: 1_000) },
                randomKey: { Data(repeating: 7, count: 32) })
            let material = try store.prepare(for: credential())
            try store.recordCompanion(deviceID: "watch-1", generation: material.generation)

            let first = try store.beginWipe(for: credential())
            let duplicate = try store.beginWipe(for: credential())
            let reloaded = PhoneWatchControlStore(keychain: keychain)

            #expect(first == duplicate)
            #expect(try reloaded.pendingWipes() == [first])
            #expect(first.companionDeviceID == "watch-1")
        }

        @Test("Only an authenticated matching ack clears pending retry")
        func authenticatedAckClears() throws {
            let keychain = InMemoryKeychain()
            let store = PhoneWatchControlStore(
                keychain: keychain,
                now: { Date(timeIntervalSince1970: 1_000) },
                randomKey: { Data(repeating: 8, count: 32) })
            _ = try store.prepare(for: credential())
            let plan = try store.beginWipe(for: credential())

            var forged = WatchWipeAcknowledgement.authenticated(
                command: plan.command, material: plan.material)
            forged.authenticationTag = "forged"
            #expect(try !store.acknowledge(forged))
            #expect(try store.pendingWipes().count == 1)

            let valid = WatchWipeAcknowledgement.authenticated(
                command: plan.command, material: plan.material)
            #expect(try store.acknowledge(valid))
            #expect(try store.pendingWipes().isEmpty)
        }

        @Test("An acknowledged wipe generation is never reused")
        func acknowledgedGenerationIsRetired() throws {
            let keychain = InMemoryKeychain()
            let store = PhoneWatchControlStore(
                keychain: keychain,
                now: { Date(timeIntervalSince1970: 1_000) },
                randomKey: { Data(repeating: 9, count: 32) })
            let firstMaterial = try store.prepare(for: credential())
            let firstPlan = try store.beginWipe(for: credential())
            let acknowledgement = WatchWipeAcknowledgement.authenticated(
                command: firstPlan.command, material: firstPlan.material)
            #expect(try store.acknowledge(acknowledgement))

            let nextMaterial = try store.prepare(for: credential())

            #expect(nextMaterial.generation > firstMaterial.generation)
            #expect(nextMaterial.authenticationKey == Data(repeating: 9, count: 32))
        }

        @Test("A newer active generation gets its own wipe while an old one is pending")
        func multiplePendingGenerationsRemainDistinct() throws {
            let keychain = InMemoryKeychain()
            let store = PhoneWatchControlStore(
                keychain: keychain,
                now: { Date(timeIntervalSince1970: 1_000) },
                randomKey: { Data(repeating: 10, count: 32) })
            _ = try store.prepare(for: credential())
            let first = try store.beginWipe(for: credential())

            _ = try store.prepare(for: credential())
            let second = try store.beginWipe(for: credential())

            #expect(first.command.commandID != second.command.commandID)
            #expect(second.command.generation > first.command.generation)
            #expect(try store.pendingWipes() == [first, second])
        }
    }
#endif
