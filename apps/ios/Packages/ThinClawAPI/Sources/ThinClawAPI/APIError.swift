import Foundation

/// Error taxonomy for gateway calls, mapped from transport and HTTP status
/// failures so feature code can branch on semantics rather than raw codes.
public enum APIError: Error, Sendable, Equatable {
    /// No device credential is stored — onboarding has not completed.
    case notPaired
    /// 401 — the device token was rejected (revoked, rotated, or expired).
    case unauthorized
    /// 403 — the device token lacks the required scope.
    case forbidden
    /// 429 — chat rate limit; retry after the given interval when known.
    case rateLimited(retryAfter: TimeInterval?)
    /// 5xx from the gateway.
    case server(status: Int)
    /// The endpoint was reachable but TLS identity verification failed
    /// (SPKI pin mismatch). Never retried automatically.
    case pinMismatch
    /// Network-level failure (unreachable, timed out, connection lost).
    case transport(URLError.Code)
    /// Anything else, preserved for diagnostics.
    case unexpected(status: Int)

    public init(status: Int, retryAfter: TimeInterval? = nil) {
        switch status {
        case 401: self = .unauthorized
        case 403: self = .forbidden
        case 429: self = .rateLimited(retryAfter: retryAfter)
        case 500...599: self = .server(status: status)
        default: self = .unexpected(status: status)
        }
    }
}
