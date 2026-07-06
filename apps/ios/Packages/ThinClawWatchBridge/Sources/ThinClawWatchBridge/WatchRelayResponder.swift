#if canImport(Security) && canImport(CryptoKit)
    import Foundation

    /// Answers a decoded ``WatchRelayEnvelope`` by forwarding it to the gateway
    /// with the **watch's own token** (D-K4) and mapping the outcome to a
    /// ``WatchRelayResponse``.
    ///
    /// This is the pure core of the relay host: it holds no WatchConnectivity
    /// state, so the whole forward-and-map flow — including the invariant that a
    /// relayed approval authenticates with the watch token and never the phone's
    /// — is exercised by plain `swift test` on macOS with a stub gateway. The
    /// `WCSessionDelegate` (`WatchRelayHost`) is a thin transport shell over this.
    public struct WatchRelayResponder: Sendable {
        private let gateway: any WatchBridgeGateway

        public init(gateway: any WatchBridgeGateway) {
            self.gateway = gateway
        }

        /// Forward `envelope` and return the response to relay back to the watch.
        ///
        /// `phoneToken` is accepted only to make explicit that it is **not** used
        /// to authenticate a relayed request — the watch token inside the
        /// envelope is. A request whose envelope carries no watch token cannot be
        /// attributed to the watch, so it fails closed with
        /// ``WatchRelayResponse/reprovisionRequired`` rather than silently
        /// falling back to the phone's credential.
        public func answer(
            _ envelope: WatchRelayEnvelope,
            phoneToken: String
        ) async -> WatchRelayResponse {
            switch envelope.request {
            case .approve(let requestID, let threadID, let action):
                guard let watchToken = envelope.watchToken else {
                    return .reprovisionRequired
                }
                return await forward(watchToken: watchToken) {
                    try await gateway.forwardApproval(
                        watchToken: watchToken, requestID: requestID,
                        threadID: threadID, action: action)
                    return .accepted
                }

            case .quickAsk(let prompt, let threadID):
                guard let watchToken = envelope.watchToken else {
                    return .reprovisionRequired
                }
                return await forward(watchToken: watchToken) {
                    let messageID = try await gateway.forwardQuickAsk(
                        watchToken: watchToken, prompt: prompt, threadID: threadID)
                    return .accepted(messageID: messageID)
                }

            case .snapshotRefresh:
                // Snapshot delivery is a push from the host, not a request
                // outcome; acknowledge so the watch knows the ask landed and can
                // await the next application-context update.
                return .accepted
            }
        }

        // Run a forwarding closure, translating a revoked/unauthorized companion
        // into `reprovisionRequired` and any other failure into a short,
        // non-secret reason for the wrist.
        private func forward(
            watchToken _: String,
            _ body: () async throws -> WatchRelayResponse
        ) async -> WatchRelayResponse {
            do {
                return try await body()
            } catch let error as WatchBridgeGatewayError {
                if error.indicatesRevokedCompanion { return .reprovisionRequired }
                return .failed(reason: Self.reason(for: error))
            } catch {
                return .failed(reason: "unavailable")
            }
        }

        private static func reason(for error: WatchBridgeGatewayError) -> String {
            switch error {
            case .noReachableGateway: return "no-gateway"
            case .gateway(let apiError):
                switch apiError {
                case .rateLimited: return "rate-limited"
                case .server: return "server-error"
                case .transport: return "unreachable"
                default: return "rejected"
                }
            }
        }
    }
#endif
