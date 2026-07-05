#if canImport(Network)
    import Foundation
    import Network

    /// Discovers ThinClaw gateways advertising `_thinclaw._tcp` on the local
    /// network (used during onboarding as an alternative to QR pairing).
    ///
    /// R0 skeleton: wraps `NWBrowser` and republishes result sets as an
    /// `AsyncStream`. Resolution of an endpoint to a usable base URL (and
    /// TXT-record handling for port/TLS hints) lands with the onboarding
    /// flow at M1.
    public final class BonjourBrowser: @unchecked Sendable {
        /// The Bonjour service type ThinClaw gateways advertise.
        public static let serviceType = "_thinclaw._tcp"

        /// A gateway seen on the local network (unresolved).
        public struct DiscoveredGateway: Sendable, Hashable {
            /// Bonjour service instance name (usually the gateway name).
            public var name: String
            /// Debug description of the endpoint (host resolution is M1).
            public var endpointDescription: String
        }

        // Lock-guarded mutable state; NWBrowser itself is confined to
        // `queue`. @unchecked Sendable is sound under that discipline.
        private let lock = NSLock()
        private var browser: NWBrowser?
        private let queue = DispatchQueue(label: "com.thinclaw.ios.bonjour-browser")

        public init() {}

        /// Start browsing. Each element is the *complete current set* of
        /// discovered gateways (not a delta). The stream ends on `stop()`
        /// or browser failure.
        public func gatewaySets() -> AsyncStream<[DiscoveredGateway]> {
            AsyncStream { continuation in
                let browser = NWBrowser(
                    for: .bonjour(type: Self.serviceType, domain: nil),
                    using: NWParameters())

                browser.browseResultsChangedHandler = { results, _ in
                    let gateways = results.map { result in
                        DiscoveredGateway(
                            name: Self.instanceName(of: result.endpoint),
                            endpointDescription: String(describing: result.endpoint))
                    }
                    continuation.yield(gateways.sorted { $0.name < $1.name })
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

        public func stop() {
            let browser = lock.withLock {
                defer { self.browser = nil }
                return self.browser
            }
            browser?.cancel()
        }

        private static func instanceName(of endpoint: NWEndpoint) -> String {
            if case .service(let name, _, _, _) = endpoint {
                return name
            }
            return String(describing: endpoint)
        }
    }
#endif
