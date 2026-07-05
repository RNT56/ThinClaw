import Foundation

/// Why a pairing URL was rejected.
public enum PairingPayloadError: Error, Equatable {
    /// Not a `thinclaw://pair?...` URL at all.
    case notAPairingURL
    /// Missing/undecodable `d` parameter or invalid JSON inside it.
    case malformedPayload
    /// Payload `v` is a version this client does not understand.
    case unsupportedVersion(Int)
    /// Payload `exp` is in the past.
    case expired(Date)
    /// `urls` was empty or contained no usable http(s) URL.
    case noUsableURLs
}

/// The payload embedded in a ThinClaw pairing link / QR code:
///
///     thinclaw://pair?d=<base64url(JSON)>
///
/// JSON shape (v1):
///
///     {
///       "v": 1,
///       "urls": ["https://gw.example.ts.net", "http://192.168.1.20:3000"],
///       "fp": "qMnE3hSuF3zXV0AJIT9cKW0eGD6dV3nCFbYbBGDs0XU",  // optional TLS fingerprint pin (bare base64url SHA-256 of the TLS SPKI, no prefix)
///       "iid": "inst_9f8e",          // gateway installation id
///       "name": "home-server",       // human-readable gateway name
///       "sec": "pair_5f4dcc3b…",     // one-time pairing secret
///       "exp": 1750000000            // unix seconds expiry
///     }
///
/// Unknown *extra* fields are ignored (forward compatible); an unknown `v`
/// is rejected (the payload semantics may have changed incompatibly).
public struct PairingPayload: Sendable, Equatable {
    public static let expectedScheme = "thinclaw"
    public static let expectedHost = "pair"
    public static let supportedVersion = 1

    public var version: Int
    /// Candidate gateway base URLs, in the server's preferred order,
    /// filtered to http(s) with a host.
    public var urls: [URL]
    /// Optional pinned TLS certificate fingerprint.
    public var fingerprint: String?
    /// Gateway installation id.
    public var installationID: String
    /// Human-readable gateway name (shown during onboarding).
    public var name: String
    /// One-time pairing secret, exchanged at `/api/devices/pair/complete`.
    public var secret: String
    public var expiresAt: Date

    /// Parse and validate a pairing URL.
    ///
    /// - Parameters:
    ///   - url: the full `thinclaw://pair?d=...` URL.
    ///   - now: injected clock for expiry checks (tests pin this).
    public static func parse(from url: URL, now: Date = Date()) throws -> PairingPayload {
        guard
            url.scheme?.lowercased() == expectedScheme,
            url.host?.lowercased() == expectedHost,
            let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
            let encoded = components.queryItems?.first(where: { $0.name == "d" })?.value,
            !encoded.isEmpty
        else {
            throw PairingPayloadError.notAPairingURL
        }

        guard
            let data = Data(base64URLEncoded: encoded),
            let wire = try? JSONDecoder().decode(WirePayload.self, from: data)
        else {
            throw PairingPayloadError.malformedPayload
        }

        guard wire.v == supportedVersion else {
            throw PairingPayloadError.unsupportedVersion(wire.v)
        }

        let expiresAt = Date(timeIntervalSince1970: TimeInterval(wire.exp))
        guard expiresAt > now else {
            throw PairingPayloadError.expired(expiresAt)
        }

        let urls = wire.urls.compactMap { raw -> URL? in
            guard
                let candidate = URL(string: raw),
                let scheme = candidate.scheme?.lowercased(),
                scheme == "http" || scheme == "https",
                candidate.host != nil
            else { return nil }
            return candidate
        }
        guard !urls.isEmpty else {
            throw PairingPayloadError.noUsableURLs
        }

        return PairingPayload(
            version: wire.v,
            urls: urls,
            fingerprint: wire.fp,
            installationID: wire.iid,
            name: wire.name,
            secret: wire.sec,
            expiresAt: expiresAt)
    }

    public init(
        version: Int,
        urls: [URL],
        fingerprint: String?,
        installationID: String,
        name: String,
        secret: String,
        expiresAt: Date
    ) {
        self.version = version
        self.urls = urls
        self.fingerprint = fingerprint
        self.installationID = installationID
        self.name = name
        self.secret = secret
        self.expiresAt = expiresAt
    }
}

/// The literal JSON wire shape. Extra unknown fields are ignored because
/// `JSONDecoder` only reads declared keys.
private struct WirePayload: Decodable {
    let v: Int
    let urls: [String]
    let fp: String?
    let iid: String
    let name: String
    let sec: String
    let exp: Int
}

extension Data {
    /// Decode base64url (RFC 4648 §5) with or without padding.
    public init?(base64URLEncoded input: String) {
        var base64 =
            input
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        let remainder = base64.count % 4
        if remainder == 2 {
            base64 += "=="
        } else if remainder == 3 {
            base64 += "="
        } else if remainder == 1 {
            return nil  // never a valid base64 length
        }
        self.init(base64Encoded: base64)
    }

    /// Encode to base64url without padding (used by tests and QR previews).
    public func base64URLEncodedString() -> String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
