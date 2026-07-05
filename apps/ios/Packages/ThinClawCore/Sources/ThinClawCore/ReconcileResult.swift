import Foundation

/// The repair set produced by reconciling a thread's local transcript against
/// the gateway's authoritative history head after a reconnect.
///
/// The gateway SSE stream has **no replay**: events missed while the
/// connection was down are gone. After reconnecting, the client refetches the
/// most recent history and diffs it against what it already holds locally,
/// producing this minimal set of edits to bring the local view back in sync.
///
/// A caller applies it as: drop nothing that is not in ``removed``, upsert
/// every item in ``upserted`` (by ``TimelineItem/id``), then trust the merged
/// ordering by ``TimelineItem/timestamp``.
public struct ReconcileResult: Hashable, Sendable {
    /// The thread that was reconciled.
    public var threadID: ThreadID
    /// Items present on the server that the local view is missing or that
    /// changed (matched by id); apply these as inserts-or-updates.
    public var upserted: [TimelineItem]
    /// Ids the local view holds that the server's history head no longer
    /// contains within the reconciled window; drop these.
    public var removed: [MessageID]

    /// Whether reconciliation found any divergence at all.
    public var isEmpty: Bool { upserted.isEmpty && removed.isEmpty }

    public init(
        threadID: ThreadID,
        upserted: [TimelineItem] = [],
        removed: [MessageID] = []
    ) {
        self.threadID = threadID
        self.upserted = upserted
        self.removed = removed
    }

    /// Diff `local` items against the authoritative `server` history head for
    /// a thread, returning the minimal repair set.
    ///
    /// Matching is by ``TimelineItem/id``. An id present on both sides but
    /// with a different value (e.g. a tool call that flipped from running to
    /// succeeded, or a streaming message that finalized) is upserted with the
    /// server's version. An id only on the server is inserted. An id only in
    /// `local` — but within the timestamp window the server covers — is
    /// removed; local items older than the server's window are left untouched
    /// (the server only returned a recent head, not the whole thread).
    public static func diff(
        threadID: ThreadID,
        local: [TimelineItem],
        server: [TimelineItem]
    ) -> ReconcileResult {
        var serverByID: [MessageID: TimelineItem] = [:]
        serverByID.reserveCapacity(server.count)
        for item in server where item.threadID == threadID {
            serverByID[item.id] = item
        }

        var localByID: [MessageID: TimelineItem] = [:]
        localByID.reserveCapacity(local.count)
        for item in local where item.threadID == threadID {
            localByID[item.id] = item
        }

        // The window the server's head actually covers, scoped to the
        // requested thread. Local items older than this are not "missing" —
        // they are simply not in this page — so they must never be removed.
        //
        // Defense-in-depth: a misrouted server response carrying items for a
        // *different* thread must never define this window. If it did, an
        // empty-for-this-thread head with older cross-thread timestamps would
        // make every recent local item look "missing" and get deleted. Scoping
        // to `threadID` makes a wrong-thread response reconcile to a no-op for
        // this thread rather than wiping its local transcript.
        let windowStart =
            server
            .filter { $0.threadID == threadID }
            .map(\.timestamp)
            .min()

        var upserted: [TimelineItem] = []
        for item in server where item.threadID == threadID {
            if localByID[item.id] != item {
                upserted.append(item)
            }
        }

        var removed: [MessageID] = []
        // An empty server head defines no window, so there is nothing to
        // reconcile against — trust the local view wholesale rather than
        // deleting it.
        if let windowStart {
            for item in local where item.threadID == threadID {
                guard serverByID[item.id] == nil else { continue }
                if item.timestamp < windowStart { continue }
                removed.append(item.id)
            }
        }

        return ReconcileResult(threadID: threadID, upserted: upserted, removed: removed)
    }
}
