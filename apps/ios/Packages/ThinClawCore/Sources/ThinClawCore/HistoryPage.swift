import Foundation

/// One page of transcript history for a thread, as returned by
/// `GET /api/chat/history`.
///
/// The gateway paginates oldest-first with a timestamp cursor: `items` holds
/// the turns decoded into flat ``TimelineItem`` rows, and ``oldestTimestamp``
/// is the cursor to pass as `before:` for the next (older) page when
/// ``hasMore`` is true.
public struct HistoryPage: Hashable, Sendable {
    /// The thread this page belongs to.
    public var threadID: ThreadID
    /// Timeline rows for this page, ordered oldest-first.
    public var items: [TimelineItem]
    /// Whether older pages remain behind this one.
    public var hasMore: Bool
    /// Cursor (timestamp of the oldest item on this page) for the next
    /// `before:` request, when the gateway supplied one.
    public var oldestTimestamp: Date?

    public init(
        threadID: ThreadID,
        items: [TimelineItem],
        hasMore: Bool,
        oldestTimestamp: Date? = nil
    ) {
        self.threadID = threadID
        self.items = items
        self.hasMore = hasMore
        self.oldestTimestamp = oldestTimestamp
    }
}
