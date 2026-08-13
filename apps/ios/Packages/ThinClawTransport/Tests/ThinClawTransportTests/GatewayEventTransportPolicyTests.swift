import Foundation
import Testing

@testable import ThinClawTransport

@Suite("Gateway event transport policy")
struct GatewayEventTransportPolicyTests {
    @Test("mobile standard is one explicit SSE policy")
    func mobileStandard() {
        #expect(GatewayEventTransportPolicy.allCases == [.serverSentEvents])
        #expect(GatewayEventTransportPolicy.mobileDefault == .serverSentEvents)
        #expect(
            GatewayEventTransportPolicy.mobileDefault.missedEventRecovery
                == .reconcileRESTSnapshots)
        #expect(
            GatewayEventTransportPolicy.mobileDefault.failurePolicy
                == .reconnectSameTransport)
    }

    @Test("request is pinned-session compatible and keeps credentials out of URL")
    func requestContract() throws {
        let baseURL = try #require(URL(string: "https://gateway.example/base"))
        let request = GatewayEventTransportPolicy.mobileDefault.request(
            baseURL: baseURL,
            token: "device-secret"
        )

        #expect(request.httpMethod == "GET")
        #expect(request.url?.path == "/base/api/chat/events")
        #expect(request.url?.query == nil)
        #expect(request.value(forHTTPHeaderField: "Accept") == "text/event-stream")
        #expect(
            request.value(forHTTPHeaderField: "Authorization")
                == "Bearer device-secret")
    }
}
