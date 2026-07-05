import Foundation
import Observation
import ThinClawCore
import ThinClawPersistence
import ThinClawTransport

/// Sessions surface view model: cache-first hydrate, then a network refresh via
/// ``GatewaySession/threads()``. The ordering/merge logic is the pure
/// ``SessionsListModel`` (unit-tested on macOS); this shell owns the async
/// effects and SwiftUI observation.
@MainActor
@Observable
public final class SessionsStore {
    /// The thread rows, most-recently-updated first.
    public private(set) var threads: [ChatThread] = []
    /// True after the first successful network refresh, so the UI can tell an
    /// empty-but-loading list from a genuinely empty one.
    public private(set) var hasRefreshed: Bool = false

    private let session: GatewaySession
    private let store: any TranscriptStoring
    private var model = SessionsListModel()

    public init(session: GatewaySession, store: any TranscriptStoring) {
        self.session = session
        self.store = store
    }

    /// Paint from the local cache immediately, then refresh from the gateway.
    public func load() async {
        await hydrateFromCache()
        await refresh()
    }

    /// Hydrate the visible list from the local cache for instant first paint.
    public func hydrateFromCache() async {
        let cached = (try? await store.threads()) ?? []
        model.hydrate(cached: cached)
        publish()
    }

    /// Refresh from the gateway and persist the authoritative listing back into
    /// the cache. A failure leaves the cached rows on screen.
    public func refresh() async {
        guard let fetched = try? await session.threads() else { return }
        model.refresh(fetched: fetched)
        publish()
        // Warm the cache for next launch.
        for thread in fetched {
            try? await store.upsert(thread: thread)
        }
    }

    private func publish() {
        threads = model.threads
        hasRefreshed = model.hasRefreshed
    }
}
