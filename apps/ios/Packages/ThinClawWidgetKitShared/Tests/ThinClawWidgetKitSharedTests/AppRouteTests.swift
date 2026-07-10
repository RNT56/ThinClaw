import Foundation
import Testing

@testable import ThinClawWidgetKitShared

@Suite("AppRoute")
struct AppRouteTests {
    @Test(
        "canonical routes round-trip",
        arguments: [
            AppRoute.thread("thread/with space"),
            AppRoute.approvals(requestID: "approval-1", threadID: "thread-1"),
            AppRoute.approvals(requestID: nil, threadID: nil),
            AppRoute.job("job-1"),
            AppRoute.quickAsk,
        ])
    func canonicalRoundTrip(_ route: AppRoute) {
        #expect(AppRoute(url: route.url) == route)
    }

    @Test("legacy approve query remains accepted")
    func legacyApproval() {
        let route = AppRoute(
            url: URL(string: "thinclaw://approve?request=req-1&thread=thread-2")!)
        #expect(route == .approvals(requestID: "req-1", threadID: "thread-2"))
    }

    @Test("pair URLs preserve their redemption payload")
    func pairingURL() {
        let url = URL(string: "thinclaw://pair?d=opaque")!
        #expect(AppRoute(url: url) == .pair(url))
    }

    @Test("foreign schemes and unknown hosts are rejected")
    func rejectsInvalidRoutes() {
        #expect(AppRoute(url: URL(string: "https://example.com/thread/1")!) == nil)
        #expect(AppRoute(url: URL(string: "thinclaw://unknown")!) == nil)
    }
}
