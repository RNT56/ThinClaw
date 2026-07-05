import Foundation

/// Errors surfaced by ``SnapshotStore``.
public enum SnapshotStoreError: Error, Equatable {
    /// The snapshot on disk was written by a newer schema than this reader
    /// understands. Readers should fall back to placeholder content.
    case unsupportedSchemaVersion(found: Int, supported: Int)
    /// The file existed but could not be decoded (corrupt or truncated).
    case corruptSnapshot(fileName: String)
    /// NSFileCoordinator reported a coordination failure.
    case coordinationFailed(String)
}

/// File-based store for ``SharedSnapshot`` values inside an App Group
/// container (or any directory — the base URL is injectable so tests use a
/// temporary directory and never need entitlements).
///
/// Writes are atomic and reads/writes are wrapped in `NSFileCoordinator`,
/// because the app process and extension processes (widgets, intents) can
/// touch the same files concurrently.
public struct SnapshotStore: Sendable {
    /// Directory that holds all snapshot files.
    public let baseURL: URL

    /// Store rooted at an explicit directory (used by tests and previews).
    public init(baseURL: URL) {
        self.baseURL = baseURL
    }

    /// Store rooted at `<app group container>/Snapshots`.
    ///
    /// Returns `nil` when the container is unavailable — e.g. the app group
    /// entitlement is missing, or the code runs in a plain macOS test host.
    public init?(appGroupID: String) {
        guard
            let container = FileManager.default.containerURL(
                forSecurityApplicationGroupIdentifier: appGroupID)
        else { return nil }
        self.baseURL = container.appendingPathComponent("Snapshots", isDirectory: true)
    }

    /// Atomically persist a snapshot to its well-known file.
    public func save<S: SharedSnapshot>(_ snapshot: S) throws {
        try FileManager.default.createDirectory(
            at: baseURL, withIntermediateDirectories: true)
        let data = try Self.encoder.encode(snapshot)
        let url = fileURL(for: S.self)

        var coordinationError: NSError?
        var writeResult: Result<Void, any Error> = .success(())
        NSFileCoordinator(filePresenter: nil).coordinate(
            writingItemAt: url, options: .forReplacing, error: &coordinationError
        ) { actualURL in
            writeResult = Result { try data.write(to: actualURL, options: .atomic) }
        }
        if let coordinationError {
            throw SnapshotStoreError.coordinationFailed(coordinationError.localizedDescription)
        }
        try writeResult.get()
    }

    /// Load a snapshot; `nil` when the file does not exist yet.
    ///
    /// Throws ``SnapshotStoreError/unsupportedSchemaVersion(found:supported:)``
    /// when the file was written by a newer schema, and
    /// ``SnapshotStoreError/corruptSnapshot(fileName:)`` when undecodable.
    public func load<S: SharedSnapshot>(_ type: S.Type) throws -> S? {
        let url = fileURL(for: type)

        var coordinationError: NSError?
        var readResult: Result<Data?, any Error> = .success(nil)
        NSFileCoordinator(filePresenter: nil).coordinate(
            readingItemAt: url, options: [], error: &coordinationError
        ) { actualURL in
            readResult = Result {
                guard FileManager.default.fileExists(atPath: actualURL.path) else {
                    return nil
                }
                return try Data(contentsOf: actualURL)
            }
        }
        if let coordinationError {
            // A missing file can also surface as a coordination error
            // depending on platform; treat it as "no snapshot yet".
            if !FileManager.default.fileExists(atPath: url.path) { return nil }
            throw SnapshotStoreError.coordinationFailed(coordinationError.localizedDescription)
        }
        guard let data = try readResult.get() else { return nil }

        // Probe the version before full decode so a newer-schema file fails
        // with a precise, recoverable error instead of a random key miss.
        guard
            let probe = try? Self.decoder.decode(VersionProbe.self, from: data)
        else {
            throw SnapshotStoreError.corruptSnapshot(fileName: S.fileName)
        }
        guard probe.schemaVersion <= S.currentSchemaVersion else {
            throw SnapshotStoreError.unsupportedSchemaVersion(
                found: probe.schemaVersion, supported: S.currentSchemaVersion)
        }

        do {
            return try Self.decoder.decode(S.self, from: data)
        } catch {
            throw SnapshotStoreError.corruptSnapshot(fileName: S.fileName)
        }
    }

    /// Delete a snapshot file if present.
    public func remove<S: SharedSnapshot>(_ type: S.Type) throws {
        let url = fileURL(for: type)
        guard FileManager.default.fileExists(atPath: url.path) else { return }
        try FileManager.default.removeItem(at: url)
    }

    /// The on-disk location for a snapshot type.
    public func fileURL<S: SharedSnapshot>(for type: S.Type) -> URL {
        baseURL.appendingPathComponent(S.fileName, isDirectory: false)
    }

    // MARK: - Coding

    // ISO-8601 with fractional seconds + sorted keys: stable, diffable,
    // human-inspectable snapshot files.
    private static var encoder: JSONEncoder {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .prettyPrinted]
        encoder.dateEncodingStrategy = .custom { date, encoder in
            var container = encoder.singleValueContainer()
            try container.encode(Self.iso8601.string(from: date))
        }
        return encoder
    }

    private static var decoder: JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { decoder in
            let raw = try decoder.singleValueContainer().decode(String.self)
            if let date = Self.iso8601.date(from: raw) { return date }
            throw DecodingError.dataCorrupted(
                DecodingError.Context(
                    codingPath: decoder.codingPath,
                    debugDescription: "unparseable ISO-8601 date: \(raw)"))
        }
        return decoder
    }

    private static var iso8601: ISO8601DateFormatter {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter
    }
}

private struct VersionProbe: Decodable {
    let schemaVersion: Int
}
