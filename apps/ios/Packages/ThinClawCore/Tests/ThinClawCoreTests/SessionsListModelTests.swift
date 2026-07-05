import Foundation
import Testing

@testable import ThinClawCore

@Suite("SessionsListModel")
struct SessionsListModelTests {
    private func thread(_ id: String, updatedAt: TimeInterval) -> ChatThread {
        ChatThread(
            id: ThreadID(id), title: id,
            createdAt: Date(timeIntervalSince1970: 0),
            updatedAt: Date(timeIntervalSince1970: updatedAt))
    }

    @Test("hydrate shows cached rows first, ordered newest-first, not yet refreshed")
    func hydrateShowsCache() {
        var model = SessionsListModel()
        model.hydrate(cached: [thread("a", updatedAt: 100), thread("b", updatedAt: 200)])
        #expect(model.threads.map(\.id.rawValue) == ["b", "a"])
        #expect(!model.hasRefreshed)
    }

    @Test("refresh replaces cache with the authoritative server listing")
    func refreshReplaces() {
        var model = SessionsListModel()
        model.hydrate(cached: [thread("stale", updatedAt: 100)])
        model.refresh(fetched: [thread("fresh", updatedAt: 300)])
        #expect(model.threads.map(\.id.rawValue) == ["fresh"])
        #expect(model.hasRefreshed)
    }

    @Test("a thread the cache holds but the server drops is removed on refresh")
    func refreshDropsMissing() {
        var model = SessionsListModel()
        model.hydrate(cached: [thread("keep", updatedAt: 100), thread("gone", updatedAt: 200)])
        model.refresh(fetched: [thread("keep", updatedAt: 150)])
        #expect(model.threads.map(\.id.rawValue) == ["keep"])
    }

    @Test("cache-then-refresh: cached rows are visible before the network answer")
    func cacheThenRefreshSequence() {
        // Model the store's two-step effect: first the cache paints, then the
        // network overwrites. Asserting the intermediate state pins the
        // cache-first behavior.
        var model = SessionsListModel()
        model.hydrate(cached: [thread("cached", updatedAt: 100)])
        #expect(model.threads.map(\.id.rawValue) == ["cached"], "cache paints first")
        #expect(!model.hasRefreshed)

        model.refresh(fetched: [thread("live-1", updatedAt: 400), thread("live-2", updatedAt: 300)])
        #expect(model.threads.map(\.id.rawValue) == ["live-1", "live-2"])
        #expect(model.hasRefreshed)
    }

    @Test("refresh ordering is newest-first regardless of server order")
    func refreshReorders() {
        var model = SessionsListModel()
        model.refresh(fetched: [thread("old", updatedAt: 100), thread("new", updatedAt: 900)])
        #expect(model.threads.map(\.id.rawValue) == ["new", "old"])
    }
}
