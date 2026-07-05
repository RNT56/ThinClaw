import Foundation
import ThinClawSnapshotKit

/// Production ``SnapshotSink`` that persists the three snapshots through a
/// ``ThinClawSnapshotKit/SnapshotStore`` into the App Group container.
///
/// Each snapshot is written independently (the store's per-file writes are
/// already atomic under `NSFileCoordinator`); a failure on one is surfaced so
/// the publisher can decide whether to retry, but a partial set is safe because
/// every reader tolerates a momentarily-mixed generation.
public struct SnapshotStoreSink: SnapshotSink {
    private let store: SnapshotStore

    public init(store: SnapshotStore) {
        self.store = store
    }

    /// Convenience initializer rooted at an App Group container. Returns `nil`
    /// when the container is unavailable (missing entitlement, plain test host),
    /// so callers can degrade to no snapshot publishing rather than crash.
    public init?(appGroupID: String) {
        guard let store = SnapshotStore(appGroupID: appGroupID) else { return nil }
        self.store = store
    }

    public func write(_ snapshots: ProjectedSnapshots) throws {
        try store.save(snapshots.status)
        try store.save(snapshots.approvals)
        try store.save(snapshots.jobs)
    }
}
