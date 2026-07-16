import Foundation

/// A versioned, Codable snapshot written by the app process and read by
/// widget / watch processes through an App Group container.
///
/// Rules of the road:
/// - every snapshot carries `schemaVersion` + `generatedAt` so readers can
///   reject payloads from a *newer* app than themselves and can age out
///   stale data;
/// - one snapshot type == one well-known file name inside the container;
/// - snapshots are small denormalized projections — never a database.
public protocol SharedSnapshot: Codable, Sendable, Equatable {
    /// File name inside the snapshot directory (e.g. `"agent-status.json"`).
    static var fileName: String { get }
    /// The schema version this build of the code writes and understands.
    static var currentSchemaVersion: Int { get }
    /// The version stamped into this instance (compare on read).
    var schemaVersion: Int { get }
    /// When the writer produced this snapshot.
    var generatedAt: Date { get }
}

public protocol GatewayScopedSnapshot {
    var gatewayInstanceID: String? { get }
}

public protocol FreshnessAwareSnapshot {
    var isKnownStale: Bool { get }
}
