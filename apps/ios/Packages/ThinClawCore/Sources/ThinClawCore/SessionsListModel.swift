import Foundation

/// Pure, platform-neutral model of the Sessions surface's thread list, folding
/// a cache-first hydrate followed by a network refresh into the ordered rows
/// the list renders.
///
/// Lives in ThinClawCore so the cache-then-refresh sequencing is unit-tested on
/// macOS, with the iOS-only `@Observable` `SessionsStore` reduced to a thin
/// shell that pumps cached rows then fetched rows through it.
///
/// ## Contract
/// - ``hydrate(cached:)`` seeds the list from the local cache for instant first
///   paint.
/// - ``refresh(fetched:)`` replaces the list with the gateway's authoritative
///   listing — the gateway owns thread membership, so a thread the cache holds
///   but the server no longer lists is dropped.
/// - Rows are always ordered most-recently-updated first, matching the cache
///   and the Sessions list.
public struct SessionsListModel: Hashable, Sendable {
    /// The rows to render, most-recently-updated first.
    public private(set) var threads: [ChatThread]

    /// Whether a refresh has completed at least once (so the UI can distinguish
    /// "empty because still loading" from "genuinely no threads").
    public private(set) var hasRefreshed: Bool

    public init(threads: [ChatThread] = [], hasRefreshed: Bool = false) {
        self.threads = SessionsListModel.ordered(threads)
        self.hasRefreshed = hasRefreshed
    }

    /// Seed from the local cache. Does not mark the model refreshed — a network
    /// refresh is still expected.
    public mutating func hydrate(cached: [ChatThread]) {
        threads = Self.ordered(cached)
    }

    /// Replace with the gateway's authoritative listing.
    public mutating func refresh(fetched: [ChatThread]) {
        threads = Self.ordered(fetched)
        hasRefreshed = true
    }

    private static func ordered(_ threads: [ChatThread]) -> [ChatThread] {
        threads.sorted { $0.updatedAt > $1.updatedAt }
    }
}
