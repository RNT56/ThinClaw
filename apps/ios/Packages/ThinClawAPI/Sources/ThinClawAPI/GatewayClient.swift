import Foundation
import HTTPTypes
import OpenAPIRuntime
import OpenAPIURLSession

/// Convenience assembly of the generated `Client` for a paired gateway.
///
/// This is the seam between the hand-written shell (`GatewayEndpoint`,
/// `BearerTokenAuthenticator`, `APIError`) and the swift-openapi-generator
/// output under `Generated/`. It wires:
///
///   - the server URL from a `GatewayEndpoint` candidate,
///   - a `URLSession` transport,
///   - a middleware that injects the device bearer token and maps gateway
///     failure statuses to `APIError` before the typed decode runs.
///
/// The streaming endpoints (`/api/chat/events`, `/api/chat/ws`) are NOT part
/// of the generated surface — they belong to `ThinClawTransport`. See
/// `openapi/openapi-generator-config.yaml` for the REST-only filter.
public enum GatewayClient {
    /// Builds a generated `Client` targeting one gateway base URL.
    ///
    /// - Parameters:
    ///   - baseURL: The gateway base URL to target. Callers pick a candidate
    ///     from `GatewayEndpoint.baseURLs` (tailnet first, then LAN); URL
    ///     selection and TLS pinning live in the connection layer, not here.
    ///   - token: Supplies the current device bearer token, or `nil` before
    ///     pairing completes (yields `APIError.notPaired`).
    ///   - transport: The `ClientTransport` to use. **Required** — production
    ///     callers MUST build a `URLSessionTransport` over the pinned session
    ///     from `ThinClawAuth.PinnedSessionDelegate.makeSession()` so requests
    ///     go through TLS pinning and the D-X2 `ConnectionPolicy`; there is no
    ///     unpinned default that would silently bypass transport security
    ///     (docs/MOBILE_SECURITY.md D-X2). Tests inject a stub.
    public static func make(
        baseURL: URL,
        token: @escaping @Sendable () -> String?,
        transport: any ClientTransport
    ) -> Client {
        Client(
            serverURL: baseURL,
            transport: transport,
            middlewares: [BearerAuthMiddleware(token: token)]
        )
    }

    /// Builds a generated `Client` over `session`, wrapping it in a
    /// `URLSessionTransport` for the caller.
    ///
    /// Prefer this over `make(baseURL:token:transport:)` when the caller only
    /// has a `URLSession` (e.g. a pinned session from
    /// `ThinClawAuth.PinnedSessionDelegate.makeSession()`) and does not want to
    /// import `OpenAPIURLSession` just to name the transport type — the
    /// Notification Service Extension links only `ThinClawAPI` + `ThinClawAuth`.
    /// The **pinning is a property of `session`**, so callers still MUST pass a
    /// pinned session to satisfy the D-X2 policy (docs/MOBILE_SECURITY.md).
    public static func make(
        baseURL: URL,
        token: @escaping @Sendable () -> String?,
        session: URLSession
    ) -> Client {
        make(
            baseURL: baseURL,
            token: token,
            transport: URLSessionTransport(configuration: .init(session: session)))
    }

    /// Builds a generated `Client` from a `GatewayEndpoint`, targeting its
    /// first (most-preferred) base URL.
    ///
    /// - Throws: `APIError.notPaired` if the endpoint has no base URLs.
    ///
    /// `transport` is **required** for the same reason as `make(baseURL:…)`:
    /// callers supply a `URLSessionTransport` over the pinned session so the
    /// D-X2 policy is never bypassed (docs/MOBILE_SECURITY.md D-X2).
    public static func make(
        endpoint: GatewayEndpoint,
        token: @escaping @Sendable () -> String?,
        transport: any ClientTransport
    ) throws -> Client {
        guard let baseURL = endpoint.baseURLs.first else { throw APIError.notPaired }
        return make(baseURL: baseURL, token: token, transport: transport)
    }
}

/// Middleware that injects the device bearer token on outgoing requests and
/// translates recognized gateway failure statuses into `APIError`.
///
/// Auth (401/403), rate limiting (429), and server (5xx) failures are surfaced
/// as `APIError` so feature code branches on semantics instead of digging
/// through each operation's `undocumented` case. Success and accepted-status
/// responses (2xx) pass through untouched so the generated typed decode runs.
struct BearerAuthMiddleware: ClientMiddleware {
    let token: @Sendable () -> String?

    func intercept(
        _ request: HTTPRequest,
        body: HTTPBody?,
        baseURL: URL,
        operationID: String,
        next: @Sendable (HTTPRequest, HTTPBody?, URL) async throws -> (HTTPResponse, HTTPBody?)
    ) async throws -> (HTTPResponse, HTTPBody?) {
        // Device tokens are header-only by contract (MOBILE_SECURITY D-T4/T14).
        guard let token = token() else { throw APIError.notPaired }
        var request = request
        request.headerFields[.authorization] = "Bearer \(token)"

        let (response, responseBody): (HTTPResponse, HTTPBody?)
        do {
            (response, responseBody) = try await next(request, body, baseURL)
        } catch let urlError as URLError {
            throw APIError.transport(urlError.code)
        }

        let status = response.status.code
        if status >= 400 {
            throw APIError(
                status: status,
                retryAfter: Self.retryAfter(from: response)
            )
        }
        return (response, responseBody)
    }

    /// Parses a `Retry-After` delta-seconds header into a `TimeInterval`.
    /// HTTP-date form is not emitted by the gateway, so it is ignored.
    private static func retryAfter(from response: HTTPResponse) -> TimeInterval? {
        guard let raw = response.headerFields[.retryAfter],
            let seconds = TimeInterval(raw.trimmingCharacters(in: .whitespaces))
        else { return nil }
        return seconds
    }
}

extension APIError {
    /// Normalizes any error thrown by a generated client call into an
    /// `APIError`.
    ///
    /// The OpenAPI runtime wraps errors thrown from a middleware (including the
    /// `APIError` values `BearerAuthMiddleware` raises) inside a `ClientError`,
    /// and surfaces raw transport failures as `URLError`. Feature code should
    /// run every generated call through this so it always branches on a stable
    /// `APIError`, e.g.:
    ///
    /// ```swift
    /// do { return try await client.healthHandler() }
    /// catch { throw APIError.from(error) }
    /// ```
    public static func from(_ error: any Error) -> APIError {
        if let apiError = error as? APIError {
            return apiError
        }
        if let clientError = error as? ClientError {
            return from(clientError.underlyingError)
        }
        if let urlError = error as? URLError {
            return .transport(urlError.code)
        }
        return .unexpected(status: 0)
    }
}
