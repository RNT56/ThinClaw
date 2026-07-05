import Foundation

/// Where a paired ThinClaw gateway lives and how to authenticate against it.
///
/// The transport-security policy (pinned SPKI, refusal of plain HTTP off the
/// tailnet) is enforced by the connection layer that consumes this value —
/// see `docs/MOBILE_SECURITY.md` (D-X2) in the repository root.
public struct GatewayEndpoint: Sendable, Hashable {
    /// Candidate base URLs in preference order (tailnet first, then LAN).
    public var baseURLs: [URL]

    /// SHA-256 of the gateway TLS leaf's SubjectPublicKeyInfo, delivered in
    /// the pairing QR. `nil` only for the explicit `vpn-http` pairing mode.
    public var spkiPinSHA256: Data?

    /// Stable gateway instance identifier from the pairing payload, used to
    /// re-identify the instance after its addresses change.
    public var instanceID: String

    public init(baseURLs: [URL], spkiPinSHA256: Data?, instanceID: String) {
        self.baseURLs = baseURLs
        self.spkiPinSHA256 = spkiPinSHA256
        self.instanceID = instanceID
    }
}

/// Injects the device bearer token into outgoing gateway requests.
///
/// Device tokens are header-only by contract — never query parameters
/// (`docs/MOBILE_SECURITY.md`, D-T4/T14).
public struct BearerTokenAuthenticator: Sendable {
    private let token: @Sendable () -> String?

    public init(token: @escaping @Sendable () -> String?) {
        self.token = token
    }

    public func authenticate(_ request: inout URLRequest) throws {
        guard let token = token() else { throw APIError.notPaired }
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
    }
}
