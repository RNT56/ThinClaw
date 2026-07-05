import Foundation
import HTTPTypes
import OpenAPIRuntime
import Testing

@testable import ThinClawAPI

/// A `ClientTransport` that returns one canned response and captures the last
/// request it saw, so tests can exercise the generated client + middleware
/// without a live gateway.
private final class StubTransport: ClientTransport, @unchecked Sendable {
    let status: Int
    let body: Data
    let contentType: String
    let extraHeaders: [HTTPField.Name: String]
    private(set) var lastAuthorization: String?
    /// The request-target (path + query) of the last request, captured *after*
    /// the generated client serializes inputs into the URL. This is the tier a
    /// mocked `APIProtocol` bypasses, so it is where a query-vs-path parameter
    /// bug actually shows up.
    private(set) var lastPath: String?

    init(
        status: Int,
        body: Data,
        contentType: String = "application/json",
        extraHeaders: [HTTPField.Name: String] = [:]
    ) {
        self.status = status
        self.body = body
        self.contentType = contentType
        self.extraHeaders = extraHeaders
    }

    func send(
        _ request: HTTPRequest,
        body requestBody: HTTPBody?,
        baseURL: URL,
        operationID: String
    ) async throws -> (HTTPResponse, HTTPBody?) {
        lastAuthorization = request.headerFields[.authorization]
        lastPath = request.path
        var response = HTTPResponse(status: .init(code: status))
        response.headerFields[.contentType] = contentType
        for (name, value) in extraHeaders {
            response.headerFields[name] = value
        }
        return (response, HTTPBody(body))
    }
}

@Suite("Generated gateway client")
struct GeneratedClientTests {
    private static let baseURL = URL(string: "https://gateway.local")!

    // MARK: compile-level smoke

    @Test("client instantiates and injects the bearer token on a 200")
    func smokeInstantiate() async throws {
        let transport = StubTransport(
            status: 200,
            body: #"{"status":"ok","channel":"gateway"}"#.data(using: .utf8)!
        )
        let client = GatewayClient.make(
            baseURL: Self.baseURL,
            token: { "tcd_smoke" },
            transport: transport
        )

        let output = try await client.healthHandler()
        let health = try output.ok.body.json
        #expect(health.status == "ok")
        #expect(health.channel == "gateway")
        #expect(transport.lastAuthorization == "Bearer tcd_smoke")
    }

    @Test("make(endpoint:) targets the first candidate base URL")
    func endpointConvenience() throws {
        let endpoint = GatewayEndpoint(
            baseURLs: [Self.baseURL],
            spkiPinSHA256: nil,
            instanceID: "inst-1"
        )
        // Compiles and returns a usable client (no network performed).
        _ = try GatewayClient.make(
            endpoint: endpoint, token: { "t" }, transport: StubTransport(status: 200, body: Data("{}".utf8)))
    }

    @Test("make(endpoint:) with no base URLs surfaces notPaired")
    func endpointNoBaseURL() {
        let endpoint = GatewayEndpoint(baseURLs: [], spkiPinSHA256: nil, instanceID: "inst-1")
        #expect(throws: APIError.notPaired) {
            _ = try GatewayClient.make(
                endpoint: endpoint,
                token: { "t" },
                transport: StubTransport(status: 200, body: Data("{}".utf8)))
        }
    }

    // MARK: URL serialization (the tier a mocked APIProtocol bypasses)

    /// Regression guard for the CRITICAL history-pagination bug: the spec once
    /// declared thread_id/before/limit as PATH parameters, so the generated
    /// client dropped them (the path template had no placeholders). A mocked
    /// `APIProtocol` test cannot catch that — it never serializes a URL. This
    /// drives the *real* generated client through a stub `ClientTransport` and
    /// inspects the request-target after serialization.
    @Test("chat history serializes thread_id, before, and limit into the query string")
    func historyQuerySerialization() async throws {
        let transport = StubTransport(
            status: 200,
            body: #"{"thread_id":"t7","turns":[]}"#.data(using: .utf8)!
        )
        let client = GatewayClient.make(
            baseURL: Self.baseURL,
            token: { "tcd_hist" },
            transport: transport
        )

        _ = try await client.chatHistoryHandler(
            query: .init(threadId: "t7", limit: 25, before: "2026-07-04T10:00:00.000Z"))

        let path = try #require(transport.lastPath)
        // Path portion, then a query string carrying every pagination input.
        #expect(path.hasPrefix("/api/chat/history?"))
        let query = String(path.drop(while: { $0 != "?" }).dropFirst())
        let pairs = Set(query.split(separator: "&").map(String.init))
        #expect(pairs.contains("thread_id=t7"))
        #expect(pairs.contains("limit=25"))
        // The cursor is percent-encoded (":" -> "%3A") in the query.
        #expect(pairs.contains("before=2026-07-04T10%3A00%3A00.000Z"))
    }

    @Test("chat history omits absent pagination inputs from the query string")
    func historyQueryOmitsNilInputs() async throws {
        let transport = StubTransport(
            status: 200,
            body: #"{"thread_id":"t7","turns":[]}"#.data(using: .utf8)!
        )
        let client = GatewayClient.make(
            baseURL: Self.baseURL,
            token: { "tcd_hist" },
            transport: transport
        )

        _ = try await client.chatHistoryHandler(query: .init(threadId: "t7", limit: 50))

        let path = try #require(transport.lastPath)
        #expect(path.contains("thread_id=t7"))
        #expect(path.contains("limit=50"))
        // A nil cursor must not appear at all (not as an empty `before=`).
        #expect(!path.contains("before="))
    }

    // MARK: error mapping through the middleware

    /// The runtime wraps middleware-thrown errors in `ClientError`; callers
    /// unwrap via `APIError.from`. These tests assert the normalized error.
    private func expectAPIError(
        _ expected: APIError,
        from status: Int,
        headers: [HTTPField.Name: String] = [:],
        token: @escaping @Sendable () -> String? = { "t" }
    ) async {
        let client = GatewayClient.make(
            baseURL: Self.baseURL,
            token: token,
            transport: StubTransport(status: status, body: Data(), extraHeaders: headers)
        )
        do {
            _ = try await client.healthHandler()
            Issue.record("expected \(expected) but the call succeeded")
        } catch {
            #expect(APIError.from(error) == expected)
        }
    }

    @Test("401 maps to APIError.unauthorized")
    func unauthorizedMapping() async {
        await expectAPIError(.unauthorized, from: 401)
    }

    @Test("429 carries Retry-After seconds")
    func rateLimitedMapping() async {
        await expectAPIError(.rateLimited(retryAfter: 30), from: 429, headers: [.retryAfter: "30"])
    }

    @Test("5xx maps to APIError.server")
    func serverMapping() async {
        await expectAPIError(.server(status: 503), from: 503)
    }

    @Test("missing token surfaces notPaired before any transport call")
    func missingTokenMapping() async {
        await expectAPIError(.notPaired, from: 200, token: { nil })
    }

    // MARK: generated DTO decode

    @Test("HealthResponse decodes from spec-shaped fixture JSON")
    func decodeHealthResponse() throws {
        let fixture = #"{"status":"ok","channel":"gateway"}"#.data(using: .utf8)!
        let health = try JSONDecoder().decode(
            Components.Schemas.HealthResponse.self,
            from: fixture
        )
        #expect(health.status == "ok")
        #expect(health.channel == "gateway")
    }
}
