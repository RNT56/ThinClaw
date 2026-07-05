import Foundation

/// A type-erased async byte stream: an ``AsyncSequence`` of `UInt8` that
/// ``SSEClient`` can consume, hiding whether the bytes come from the network
/// or a scripted test fixture.
///
/// Erasure keeps ``GatewayStream`` and ``ByteStreamProvider`` free of a
/// generic `Bytes` parameter, which would otherwise leak through the actor's
/// public surface and every reconnect attempt.
public struct ByteStream: AsyncSequence, Sendable {
    public typealias Element = UInt8

    private let makeIterator: @Sendable () -> AsyncIterator

    /// Wrap any `Sendable` byte sequence.
    public init<S>(_ base: S) where S: AsyncSequence & Sendable, S.Element == UInt8 {
        self.makeIterator = {
            var upstream = base.makeAsyncIterator()
            return AsyncIterator { try await upstream.next() }
        }
    }

    public struct AsyncIterator: AsyncIteratorProtocol {
        private var advance: () async throws -> UInt8?

        init(_ advance: @escaping () async throws -> UInt8?) {
            self.advance = advance
        }

        public mutating func next() async throws -> UInt8? {
            try await advance()
        }
    }

    public func makeAsyncIterator() -> AsyncIterator { makeIterator() }
}

/// Opens the raw SSE byte stream for `GET {base}/api/chat/events`.
///
/// The provider is the single networking seam of ``GatewayStream``: production
/// wraps `URLSession.bytes`, and tests supply a provider that replays scripted
/// fixtures so the reconnect/watchdog state machine is exercised without a
/// socket. Each ``open(token:)`` call corresponds to one connection attempt;
/// the provider is responsible for attaching the bearer token as an
/// `Authorization` header — device tokens are header-only by contract
/// (`docs/MOBILE_SECURITY.md`, D-T4/T14) and MUST NOT be placed on the query
/// string.
public protocol ByteStreamProvider: Sendable {
    /// Open a fresh event byte stream, authenticated with `token`.
    ///
    /// - Throws: if the connection cannot be established (surfaced to the
    ///   reconnect loop, which backs off and retries).
    func open(token: String) async throws -> ByteStream
}

/// Production ``ByteStreamProvider`` backed by `URLSession.bytes`.
///
/// Connects to `GET {base}/api/chat/events` with the device bearer token in
/// the `Authorization` header. TLS pinning is enforced by the `URLSession`'s
/// delegate, configured by the connection layer that constructs this — pinning
/// is not this type's concern.
///
/// The `session` is **required**: callers MUST supply the pinned session built
/// by `ThinClawAuth.PinnedSessionDelegate.makeSession()` so every event stream
/// goes through TLS pinning and the D-X2 `ConnectionPolicy`. There is
/// deliberately no `URLSession.shared` default — an unpinned default would
/// silently bypass the transport security policy (docs/MOBILE_SECURITY.md
/// D-X2).
public struct URLSessionByteStreamProvider: ByteStreamProvider {
    private let baseURL: URL
    private let session: URLSession

    public init(baseURL: URL, session: URLSession) {
        self.baseURL = baseURL
        self.session = session
    }

    public func open(token: String) async throws -> ByteStream {
        var request = URLRequest(url: baseURL.appending(path: "api/chat/events"))
        request.httpMethod = "GET"
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        // Header-only auth (MOBILE_SECURITY D-T4/T14) — never `?token=`.
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")

        let (bytes, response) = try await session.bytes(for: request)
        if let http = response as? HTTPURLResponse, http.statusCode >= 400 {
            throw GatewayStreamError.http(status: http.statusCode)
        }
        return ByteStream(bytes)
    }
}

/// Failures opening or reading the gateway event stream.
public enum GatewayStreamError: Error, Hashable, Sendable {
    /// The event endpoint returned a non-2xx status on connect.
    case http(status: Int)
}
