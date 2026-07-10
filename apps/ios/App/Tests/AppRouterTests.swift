import Foundation
import Testing
import ThinClawCore
import ThinClawWidgetKitShared

@testable import ThinClaw

@MainActor
@Suite("AppRouter deep-link consumption")
struct AppRouterTests {
    @Test("thread route selects the thread and Chat tab")
    func threadRoute() {
        let router = AppRouter()
        router.selectedTab = .settings

        router.handle(deepLink: AppRoute.thread("thread-1").url)

        #expect(router.selectedTab == .chat)
        #expect(router.selectedThread == ThreadID("thread-1"))
    }

    @Test("approval and job routes focus their canonical tabs")
    func focusedRoutes() {
        let router = AppRouter()
        router.handle(
            deepLink: AppRoute.approvals(
                requestID: "request-1",
                threadID: "thread-1"
            ).url)
        #expect(router.selectedTab == .approvals)
        #expect(router.focusedApprovalID == "request-1")
        #expect(router.selectedThread == ThreadID("thread-1"))

        router.handle(deepLink: AppRoute.job("job-1").url)
        #expect(router.selectedTab == .jobs)
        #expect(router.focusedJobID == "job-1")
    }

    @Test("quick ask focuses Chat")
    func quickAsk() {
        let router = AppRouter()
        router.selectedTab = .jobs
        router.handle(deepLink: AppRoute.quickAsk.url)
        #expect(router.selectedTab == .chat)
    }
}
