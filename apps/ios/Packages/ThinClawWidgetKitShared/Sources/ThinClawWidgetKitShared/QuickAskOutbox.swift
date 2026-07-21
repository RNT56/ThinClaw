#if canImport(Security) && canImport(CryptoKit)
    import CryptoKit
    import Foundation
    import ThinClawAuth
    import ThinClawSnapshotKit

    public struct QuickAskQueuedPrompt: Sendable, Equatable, Identifiable {
        public var id: UUID
        public var content: String
        public var threadID: String?
        public var queuedAt: Date
    }

    public enum QuickAskOutbox {
        private static let keyName = "thinclaw.quick-ask-outbox-key.v1"

        private struct Payload: Codable {
            var content: String
            var threadID: String?
        }

        public static func enqueue(_ content: String, threadID: String?) throws {
            guard let credential = SharedGatewayConnection.loadCredential() else {
                throw WidgetGatewayCall.Failure.notPaired
            }
            let id = UUID()
            let data = try JSONEncoder().encode(Payload(content: content, threadID: threadID))
            let sealed = try AES.GCM.seal(data, using: encryptionKey())
            guard let combined = sealed.combined else {
                throw WidgetGatewayCall.Failure.gateway("unable to encrypt queued prompt")
            }
            guard let store = WidgetSnapshotAccess.store() else {
                throw WidgetGatewayCall.Failure.gateway("app group unavailable")
            }
            var queue =
                (try store.load(EncryptedQuickAskQueue.self))
                ?? EncryptedQuickAskQueue(generatedAt: .now, entries: [])
            queue.generatedAt = .now
            queue.entries.append(
                .init(
                    id: id,
                    gatewayInstanceID: credential.installationID,
                    queuedAt: .now,
                    sealedPayload: combined))
            try store.save(queue)
        }

        public static func pending(gatewayInstanceID: String) throws -> [QuickAskQueuedPrompt] {
            guard let store = WidgetSnapshotAccess.store(),
                let queue = try store.load(EncryptedQuickAskQueue.self)
            else { return [] }
            let key = try encryptionKey()
            return queue.entries
                .filter { $0.gatewayInstanceID == gatewayInstanceID }
                .compactMap { entry in
                    guard let box = try? AES.GCM.SealedBox(combined: entry.sealedPayload),
                        let clear = try? AES.GCM.open(box, using: key),
                        let payload = try? JSONDecoder().decode(Payload.self, from: clear)
                    else { return nil }
                    return QuickAskQueuedPrompt(
                        id: entry.id,
                        content: payload.content,
                        threadID: payload.threadID,
                        queuedAt: entry.queuedAt)
                }
                .sorted { $0.queuedAt < $1.queuedAt }
        }

        public static func remove(_ id: UUID) throws {
            guard let store = WidgetSnapshotAccess.store(),
                var queue = try store.load(EncryptedQuickAskQueue.self)
            else { return }
            queue.entries.removeAll { $0.id == id }
            queue.generatedAt = .now
            try store.save(queue)
        }

        public static func removeAll(gatewayInstanceID: String) throws {
            guard let store = WidgetSnapshotAccess.store(),
                var queue = try store.load(EncryptedQuickAskQueue.self)
            else { return }
            queue.entries.removeAll { $0.gatewayInstanceID == gatewayInstanceID }
            queue.generatedAt = .now
            try store.save(queue)
        }

        private static func encryptionKey() throws -> SymmetricKey {
            let keychain = SecItemKeychainStore()
            if let data = try keychain.secret(for: keyName), data.count == 32 {
                return SymmetricKey(data: data)
            }
            let key = SymmetricKey(size: .bits256)
            let data = key.withUnsafeBytes { Data($0) }
            try keychain.setSecret(
                data,
                for: keyName,
                accessibility: .afterFirstUnlockDeviceOnly)
            return key
        }
    }
#endif
