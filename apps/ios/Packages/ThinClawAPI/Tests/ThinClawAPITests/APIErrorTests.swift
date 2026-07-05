import Foundation
import Testing

@testable import ThinClawAPI

@Suite("APIError mapping")
struct APIErrorTests {
    @Test("status codes map to semantic cases")
    func statusMapping() {
        #expect(APIError(status: 401) == .unauthorized)
        #expect(APIError(status: 403) == .forbidden)
        #expect(APIError(status: 429, retryAfter: 30) == .rateLimited(retryAfter: 30))
        #expect(APIError(status: 503) == .server(status: 503))
        #expect(APIError(status: 418) == .unexpected(status: 418))
    }

    @Test("authenticator sets the bearer header")
    func bearerHeader() throws {
        let auth = BearerTokenAuthenticator(token: { "tcd_test" })
        var request = URLRequest(url: URL(string: "https://gateway.local/api/chat/send")!)
        try auth.authenticate(&request)
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer tcd_test")
    }

    @Test("missing token surfaces notPaired")
    func missingToken() {
        let auth = BearerTokenAuthenticator(token: { nil })
        var request = URLRequest(url: URL(string: "https://gateway.local/api/health")!)
        #expect(throws: APIError.notPaired) {
            try auth.authenticate(&request)
        }
    }
}
