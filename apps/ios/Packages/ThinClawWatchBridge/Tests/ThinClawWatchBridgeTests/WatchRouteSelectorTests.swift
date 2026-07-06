import Testing

@testable import ThinClawWatchBridge

@Suite("WatchRouteSelector relay→direct→queue policy")
struct WatchRouteSelectorTests {
    @Test("Relay is preferred when the phone is reachable")
    func relayPreferred() {
        let reach = WatchReachability(relayReachable: true, directReachable: true)
        #expect(WatchRouteSelector.primaryRoute(for: reach) == .relay)
        #expect(WatchRouteSelector.routeOrder(for: reach) == [.relay, .direct, .queued])
    }

    @Test("Direct is used when the phone is unreachable but the gateway is")
    func directWhenNoRelay() {
        let reach = WatchReachability(relayReachable: false, directReachable: true)
        #expect(WatchRouteSelector.primaryRoute(for: reach) == .direct)
        #expect(WatchRouteSelector.routeOrder(for: reach) == [.direct, .queued])
    }

    @Test("Queue when neither relay nor direct is available")
    func queueWhenNothingReachable() {
        let reach = WatchReachability(relayReachable: false, directReachable: false)
        #expect(WatchRouteSelector.primaryRoute(for: reach) == .queued)
        #expect(WatchRouteSelector.routeOrder(for: reach) == [.queued])
    }

    @Test("Relay timeout falls through to direct")
    func relayFallsThroughToDirect() {
        let reach = WatchReachability(relayReachable: true, directReachable: true)
        #expect(WatchRouteSelector.nextRoute(after: .relay, reachability: reach) == .direct)
    }

    @Test("Relay timeout with no direct route falls through to queue (nil)")
    func relayFallsThroughToQueue() {
        let reach = WatchReachability(relayReachable: true, directReachable: false)
        // Only relay + queued in the order; the next hop is the terminal queue,
        // surfaced as nil so the caller queues rather than trying another live
        // route.
        #expect(WatchRouteSelector.nextRoute(after: .relay, reachability: reach) == nil)
    }

    @Test("Direct timeout falls through to queue (nil)")
    func directFallsThroughToQueue() {
        let reach = WatchReachability(relayReachable: false, directReachable: true)
        #expect(WatchRouteSelector.nextRoute(after: .direct, reachability: reach) == nil)
    }

    @Test("queued is terminal — never a next route")
    func queuedIsTerminal() {
        let reach = WatchReachability(relayReachable: true, directReachable: true)
        #expect(WatchRouteSelector.nextRoute(after: .queued, reachability: reach) == nil)
    }

    @Test("Approval budget accommodates a relay attempt plus a direct fallback")
    func timingBudgetFitsFallthrough() {
        // Two sequential route attempts must still fit inside the < 5s
        // interactive approval target from the brief.
        #expect(WatchRelayTiming.routeDeadline * 2 <= WatchRelayTiming.approvalBudget)
    }
}
