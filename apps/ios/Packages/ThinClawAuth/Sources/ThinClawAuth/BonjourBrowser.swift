import Foundation

/// A ThinClaw gateway seen on the local network (milestone B3). Purely a
/// *locator* candidate: it names where a gateway might be reachable, never
/// whether it is the trusted one. Selecting it may pre-fill the pairing URL,
/// but pairing still requires the QR secret (or typed code) and the connection
/// still verifies the pinned SPKI + instance id before any credential is sent
/// (docs/MOBILE_SECURITY.md D-X3 / T11).
public struct DiscoveredGateway: Sendable, Hashable, Identifiable {
    /// Stable identity for SwiftUI lists: the Bonjour service instance name is
    /// unique on a given network.
    public var id: String { name }

    /// Bonjour service instance name (advertised display label).
    public var name: String
    /// Resolved host (an IP literal or `.local` name), when resolution
    /// succeeded; `nil` while the endpoint is still unresolved.
    public var host: String?
    /// Resolved port, when resolution succeeded.
    public var port: Int?
    /// Parsed locator hints from the TXT record (version/api/name/fp).
    public var txt: DiscoveryTXTRecord
    /// Debug description of the underlying endpoint (diagnostics only).
    public var endpointDescription: String

    public init(
        name: String,
        host: String? = nil,
        port: Int? = nil,
        txt: DiscoveryTXTRecord = DiscoveryTXTRecord(),
        endpointDescription: String = ""
    ) {
        self.name = name
        self.host = host
        self.port = port
        self.txt = txt
        self.endpointDescription = endpointDescription
    }

    /// The human-facing display name: prefer the TXT `name` hint, fall back to
    /// the Bonjour instance name.
    public var displayName: String {
        txt.name ?? name
    }

    /// The non-reversible gateway instance fingerprint (`fp`) advertised in the
    /// TXT record, if any. A locator hint only — see ``DiscoveryTXTRecord``.
    public var instanceFingerprint: String? {
        txt.instanceFingerprint
    }

    /// A candidate gateway base URL to *pre-fill* the pairing form, once the
    /// endpoint has resolved to a host and port. Always `https://` because the
    /// D-X2 connection policy refuses plaintext to LAN endpoints; the advertised
    /// gateway serves its TLS listener there. Returns `nil` until resolution
    /// completes. This is a suggestion for the manual-entry field, never an
    /// authenticated endpoint.
    public var suggestedBaseURL: URL? {
        guard let host, let port else { return nil }
        return DiscoveredGateway.suggestedBaseURL(host: host, port: port)
    }

    /// Build the `https://host:port` base URL, bracketing IPv6 literals so the
    /// URL is well-formed. Split out so it is unit-testable without a network.
    public static func suggestedBaseURL(host: String, port: Int) -> URL? {
        let trimmed = host.hasSuffix(".") ? String(host.dropLast()) : host
        let bracketed = trimmed.contains(":") && !trimmed.hasPrefix("[") ? "[\(trimmed)]" : trimmed
        return URL(string: "https://\(bracketed):\(port)")
    }
}

#if canImport(Network)
    import Network

    /// Discovers ThinClaw gateways advertising `_thinclaw._tcp` on the local
    /// network, used during onboarding as a *locator* alongside QR pairing.
    ///
    /// Wraps `NWBrowser`, resolves each discovered endpoint to a host/port, and
    /// parses its TXT record into a ``DiscoveredGateway``. Result sets are
    /// republished as an `AsyncStream` (each element is the complete current
    /// set, not a delta), so appearance/disappearance is handled by simply
    /// re-emitting. Discovery never authenticates — see ``DiscoveredGateway``.
    public final class BonjourBrowser: @unchecked Sendable {
        /// The Bonjour service type ThinClaw gateways advertise.
        public static let serviceType = "_thinclaw._tcp"

        // Lock-guarded mutable state; the NWBrowser and per-connection resolves
        // are confined to `queue`. @unchecked Sendable is sound under that
        // discipline.
        private let lock = NSLock()
        private var browser: NWBrowser?
        private let queue = DispatchQueue(label: "com.thinclaw.ios.bonjour-browser")

        public init() {}

        /// Start browsing. Each yielded element is the *complete current set* of
        /// discovered gateways (not a delta). The stream ends on ``stop()`` or
        /// on unrecoverable browser failure.
        public func gatewaySets() -> AsyncStream<[DiscoveredGateway]> {
            AsyncStream { continuation in
                let parameters = NWParameters()
                parameters.includePeerToPeer = true
                let browser = NWBrowser(
                    for: .bonjour(type: Self.serviceType, domain: nil),
                    using: parameters)

                browser.browseResultsChangedHandler = { results, _ in
                    let gateways =
                        results
                        .map { Self.gateway(from: $0) }
                        .sorted {
                            $0.displayName.localizedCaseInsensitiveCompare($1.displayName)
                                == .orderedAscending
                        }
                    continuation.yield(gateways)
                }
                browser.stateUpdateHandler = { state in
                    switch state {
                    case .failed, .cancelled:
                        continuation.finish()
                    default:
                        break
                    }
                }
                continuation.onTermination = { [weak self] _ in
                    self?.stop()
                }

                lock.withLock { self.browser = browser }
                browser.start(queue: queue)
            }
        }

        /// Stop browsing and tear down the underlying `NWBrowser`. Safe to call
        /// more than once; also invoked when the stream's consumer cancels.
        public func stop() {
            let browser = lock.withLock {
                defer { self.browser = nil }
                return self.browser
            }
            browser?.cancel()
        }

        // MARK: - Endpoint mapping

        /// Map a browse result to a ``DiscoveredGateway``: the instance name,
        /// the parsed TXT record, and (when the endpoint is already a resolved
        /// host/port) the host and port. `NWBrowser` reports service endpoints
        /// pre-resolution; the host/port fields fill in once the OS resolves
        /// them, and the onboarding UI re-renders on the next emitted set.
        static func gateway(from result: NWBrowser.Result) -> DiscoveredGateway {
            let txt = parseTXT(result.metadata)
            let (name, host, port) = endpointParts(of: result.endpoint)
            return DiscoveredGateway(
                name: name,
                host: host,
                port: port,
                txt: txt,
                endpointDescription: String(describing: result.endpoint))
        }

        /// Extract the (instance name, resolved host, resolved port) from an
        /// `NWEndpoint`. A `.service` endpoint carries only the instance name
        /// (host/port unresolved ⇒ `nil`); a `.hostPort` endpoint carries the
        /// resolved address.
        static func endpointParts(of endpoint: NWEndpoint) -> (name: String, host: String?, port: Int?) {
            switch endpoint {
            case .service(let name, _, _, _):
                return (name, nil, nil)
            case .hostPort(let host, let port):
                return (hostString(host), hostString(host), Int(port.rawValue))
            default:
                return (String(describing: endpoint), nil, nil)
            }
        }

        private static func hostString(_ host: NWEndpoint.Host) -> String {
            switch host {
            case .name(let name, _):
                return name
            case .ipv4(let address):
                return "\(address)"
            case .ipv6(let address):
                return "\(address)"
            @unknown default:
                return String(describing: host)
            }
        }

        /// Decode an `NWTXTRecord` (when present) into the parsed hint model.
        /// `NWTXTRecord.dictionary` yields the string values directly (a
        /// value-less key maps to an empty string, which the parser drops).
        private static func parseTXT(_ metadata: NWBrowser.Result.Metadata) -> DiscoveryTXTRecord {
            guard case .bonjour(let record) = metadata else { return DiscoveryTXTRecord() }
            return DiscoveryTXTRecord(dictionary: record.dictionary)
        }
    }
#endif
