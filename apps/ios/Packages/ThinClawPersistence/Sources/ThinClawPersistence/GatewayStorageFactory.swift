import CryptoKit
import Foundation

/// Produces filesystem storage isolated to one authoritative gateway instance.
/// Server-provided identifiers are hashed before becoming directory names.
public struct GatewayStorageFactory {
    private let fileManager: FileManager

    public init(fileManager: FileManager = .default) {
        self.fileManager = fileManager
    }

    public func transcriptStore(
        for gatewayInstanceID: String
    ) throws -> GRDBTranscriptStore {
        try GRDBTranscriptStore.atGatewayLocation(
            installationID: gatewayInstanceID,
            fileManager: fileManager)
    }

    public func namespaceURL(for gatewayInstanceID: String) throws -> URL {
        let base = try fileManager.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true)
        let digest = SHA256.hash(data: Data(gatewayInstanceID.utf8))
            .map { String(format: "%02x", $0) }
            .joined()
        return
            base
            .appendingPathComponent("ThinClaw", isDirectory: true)
            .appendingPathComponent("Gateways", isDirectory: true)
            .appendingPathComponent(digest, isDirectory: true)
    }

    public func deleteNamespace(for gatewayInstanceID: String) throws {
        let url = try namespaceURL(for: gatewayInstanceID)
        guard fileManager.fileExists(atPath: url.path) else { return }
        try fileManager.removeItem(at: url)
    }
}
