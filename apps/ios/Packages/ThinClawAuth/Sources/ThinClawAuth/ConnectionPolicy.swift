import Foundation

/// The transport class of a candidate gateway endpoint, derived purely from
/// its scheme + host. Drives the client-side connection policy matrix
/// (docs/MOBILE_SECURITY.md **D-X2**) without any networking, so it unit-tests
/// deterministically.
public enum EndpointClass: Sendable, Hashable {
    /// Tailscale CGNAT space (`100.64.0.0/10`, `fd7a:115c:a1e0::/48`).
    case tailscale
    /// Loopback (`127.0.0.0/8`, `::1`).
    case loopback
    /// LAN / `.local` / private-range literals and any other host that is not
    /// tailnet, loopback, or a routable public address we recognize.
    case lan
    /// A routable, public host (default when nothing more specific matches a
    /// non-private literal or a public DNS name).
    case publicInternet
}

/// The verdict for a single connection attempt: whether it is allowed at all,
/// and if so whether the transport is trustworthy enough to skip the badged
/// `vpn-http` warning.
public enum ConnectionVerdict: Sendable, Hashable {
    /// Connect; the transport is authenticated (pinned SPKI or a valid
    /// public certificate chain).
    case allowedSecure
    /// Connect, but the transport is plaintext (`vpn-http` opt-in) — the UI
    /// must badge it (D-X2, threat T4).
    case allowedInsecure
    /// Refuse: the endpoint class does not permit this scheme under D-X2.
    case refused(reason: RefusalReason)

    public enum RefusalReason: Sendable, Hashable {
        /// Plain HTTP to LAN / `.local` is never allowed.
        case plainHTTPToLAN
        /// Plain HTTP to a public host is never allowed.
        case plainHTTPToPublic
        /// Plain HTTP to loopback is only allowed in DEBUG builds.
        case plainHTTPToLoopbackInRelease
        /// Scheme is neither http nor https.
        case unsupportedScheme
        /// URL had no host to classify.
        case missingHost
    }
}

/// Pure host/scheme classifier + policy evaluator for D-X2. Split from
/// ``PinnedSessionDelegate`` so the matrix is exercised in tests without a
/// live `URLSession`.
///
/// The delegate consults this to decide *whether to attempt* a connection and
/// *which trust rule* applies; the SPKI hash comparison itself lives in the
/// delegate (it needs the live server certificate).
public enum ConnectionPolicy {
    /// Classify a gateway base URL by its host.
    public static func classify(_ url: URL) -> EndpointClass? {
        guard let host = url.host?.lowercased(), !host.isEmpty else { return nil }
        return classify(host: host)
    }

    /// Classify a bare host string (no scheme). Exposed for tests.
    public static func classify(host rawHost: String) -> EndpointClass {
        // IPv6 literals arrive bracket-wrapped from `URL.host` on some
        // toolchains; normalize before range checks.
        let host = rawHost.trimmingBracketsAndZone()

        if isLoopback(host) { return .loopback }
        if isTailscale(host) { return .tailscale }
        if host.hasSuffix(".local") || isPrivateIPv4(host) || isPrivateIPv6(host) {
            return .lan
        }
        // A bare, dotless hostname (e.g. `home-server`) is a LAN name, not a
        // public DNS name.
        if !host.contains(".") && IPv4Address(host) == nil {
            return .lan
        }
        return .publicInternet
    }

    /// Evaluate D-X2 for a candidate URL.
    ///
    /// - Parameters:
    ///   - url: the gateway base URL being attempted.
    ///   - hasPin: whether a pinned SPKI fingerprint is stored for this
    ///     gateway. When a pin exists the app *never* falls back to plaintext
    ///     (D-X2 note: "never falls back from pinned TLS to HTTP").
    ///   - allowLoopbackHTTP: DEBUG-only escape hatch for `http://127.0.0.1`
    ///     during development. Production callers pass `false`.
    public static func evaluate(
        url: URL,
        hasPin: Bool,
        allowLoopbackHTTP: Bool
    ) -> ConnectionVerdict {
        guard let scheme = url.scheme?.lowercased() else {
            return .refused(reason: .unsupportedScheme)
        }
        guard scheme == "http" || scheme == "https" else {
            return .refused(reason: .unsupportedScheme)
        }
        guard let endpointClass = classify(url) else {
            return .refused(reason: .missingHost)
        }

        if scheme == "https" {
            // Pinned or public-chain TLS is accepted for every endpoint class
            // (the delegate enforces the pin when `hasPin`; standard chain
            // validation applies otherwise).
            return .allowedSecure
        }

        // scheme == "http" — plaintext. Only tailnet (vpn-http) and, in DEBUG,
        // loopback are permitted, and only when no pin is stored (a pinned
        // gateway must never be reached over plaintext).
        if hasPin {
            switch endpointClass {
            case .tailscale: return .refused(reason: .plainHTTPToPublic)
            case .loopback:
                return allowLoopbackHTTP
                    ? .allowedInsecure
                    : .refused(reason: .plainHTTPToLoopbackInRelease)
            case .lan: return .refused(reason: .plainHTTPToLAN)
            case .publicInternet: return .refused(reason: .plainHTTPToPublic)
            }
        }

        switch endpointClass {
        case .tailscale:
            return .allowedInsecure
        case .loopback:
            return allowLoopbackHTTP
                ? .allowedInsecure
                : .refused(reason: .plainHTTPToLoopbackInRelease)
        case .lan:
            return .refused(reason: .plainHTTPToLAN)
        case .publicInternet:
            return .refused(reason: .plainHTTPToPublic)
        }
    }

    // MARK: - Host classification

    private static func isLoopback(_ host: String) -> Bool {
        if host == "localhost" || host == "::1" { return true }
        if let v4 = IPv4Address(host) { return v4.octets.0 == 127 }
        return false
    }

    /// Tailscale CGNAT: IPv4 `100.64.0.0/10`, IPv6 `fd7a:115c:a1e0::/48`.
    private static func isTailscale(_ host: String) -> Bool {
        if let v4 = IPv4Address(host) {
            // 100.64.0.0/10 => first octet 100, second octet in 64...127.
            return v4.octets.0 == 100 && (64...127).contains(v4.octets.1)
        }
        if let v6 = IPv6Address(host) {
            // fd7a:115c:a1e0::/48 => first three hextets fixed.
            return v6.hextets.0 == 0xfd7a && v6.hextets.1 == 0x115c && v6.hextets.2 == 0xa1e0
        }
        return false
    }

    /// RFC 1918 private IPv4 ranges (LAN literals).
    private static func isPrivateIPv4(_ host: String) -> Bool {
        guard let v4 = IPv4Address(host) else { return false }
        let (a, b, _, _) = v4.octets
        if a == 10 { return true }
        if a == 172 && (16...31).contains(b) { return true }
        if a == 192 && b == 168 { return true }
        // Link-local 169.254.0.0/16.
        if a == 169 && b == 254 { return true }
        return false
    }

    /// IPv6 unique-local (`fc00::/7`) and link-local (`fe80::/10`) — LAN
    /// literals that are not the tailnet ULA prefix.
    private static func isPrivateIPv6(_ host: String) -> Bool {
        guard let v6 = IPv6Address(host) else { return false }
        let first = v6.hextets.0
        // fc00::/7 => top 7 bits are 1111 110x.
        if (first & 0xfe00) == 0xfc00 { return true }
        // fe80::/10 => top 10 bits are 1111 1110 10.
        if (first & 0xffc0) == 0xfe80 { return true }
        return false
    }
}

// MARK: - Minimal IP literal parsing (no Network import — keeps this pure)

/// A parsed dotted-quad IPv4 literal. Deliberately tiny: enough to bucket a
/// host into the D-X2 classes.
struct IPv4Address {
    let octets: (UInt8, UInt8, UInt8, UInt8)

    init?(_ string: String) {
        let parts = string.split(separator: ".", omittingEmptySubsequences: false)
        guard parts.count == 4 else { return nil }
        var values: [UInt8] = []
        for part in parts {
            // Reject leading '+', whitespace, or non-decimal to stay strict.
            guard part.allSatisfy(\.isNumber), let value = UInt8(part) else { return nil }
            values.append(value)
        }
        octets = (values[0], values[1], values[2], values[3])
    }
}

/// A parsed IPv6 literal, reduced to its first three 16-bit hextets — all the
/// D-X2 prefix checks need. Supports `::` compression.
struct IPv6Address {
    let hextets: (UInt16, UInt16, UInt16)

    init?(_ string: String) {
        // Strip an IPv6 zone id (`%en0`) if one slipped through.
        let core = string.split(separator: "%", maxSplits: 1).first.map(String.init) ?? string
        guard core.contains(":") else { return nil }

        let doubleColonParts = core.components(separatedBy: "::")
        guard doubleColonParts.count <= 2 else { return nil }

        func hextetList(_ segment: String) -> [UInt16]? {
            guard !segment.isEmpty else { return [] }
            var result: [UInt16] = []
            for group in segment.split(separator: ":", omittingEmptySubsequences: false) {
                guard !group.isEmpty, group.count <= 4,
                    let value = UInt16(group, radix: 16)
                else { return nil }
                result.append(value)
            }
            return result
        }

        var full: [UInt16]
        if doubleColonParts.count == 2 {
            guard let head = hextetList(doubleColonParts[0]),
                let tail = hextetList(doubleColonParts[1])
            else { return nil }
            let missing = 8 - head.count - tail.count
            guard missing >= 0 else { return nil }
            full = head + Array(repeating: 0, count: missing) + tail
        } else {
            guard let groups = hextetList(core), groups.count == 8 else { return nil }
            full = groups
        }

        guard full.count == 8 else { return nil }
        hextets = (full[0], full[1], full[2])
    }
}

extension String {
    /// Strip surrounding `[...]` from an IPv6 URL host and any `%zone` suffix.
    fileprivate func trimmingBracketsAndZone() -> String {
        var host = self
        if host.hasPrefix("[") && host.hasSuffix("]") {
            host = String(host.dropFirst().dropLast())
        }
        if let percent = host.firstIndex(of: "%") {
            host = String(host[host.startIndex..<percent])
        }
        return host
    }
}
