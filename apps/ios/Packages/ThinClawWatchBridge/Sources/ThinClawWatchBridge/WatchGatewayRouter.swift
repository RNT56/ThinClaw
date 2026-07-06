#if canImport(Security) && canImport(CryptoKit)
    import Foundation

    /// The three transports the watch can use to reach the gateway, abstracted so
    /// ``WatchGatewayProxy``'s route selection and relay→direct→queue fall-through
    /// is exercised by plain `swift test` on macOS, without WatchConnectivity or a
    /// live gateway. The concrete transports (WCSession relay, pinned URLSession,
    /// `transferUserInfo` queue) are supplied by the watch app.
    public protocol WatchRelayTransport: Sendable {
        /// Whether the phone is reachable for an interactive relay right now.
        var isReachable: Bool { get }
        /// Send `envelope` (carrying the watch token) to the phone and await its
        /// forwarded outcome. Throws ``WatchRelayError/timedOut`` past the deadline.
        func relay(_ envelope: WatchRelayEnvelope) async throws -> WatchRelayResponse
    }

    /// The direct-to-gateway transport: the watch signs the request itself with
    /// its own credential over a pinned URLSession (D-X2). No token rides on the
    /// wire — it authenticates the request as a bearer header.
    public protocol WatchDirectTransport: Sendable {
        /// Whether the watch holds a credential with a policy-allowed direct URL.
        var isReachable: Bool { get }
        /// Perform the request directly against the gateway.
        func direct(_ request: WatchRelayRequest) async throws -> WatchRelayResponse
    }

    /// The offline queue: persist an envelope for delivery when a live route
    /// returns (via `transferUserInfo`, which the OS retries). The watch surfaces
    /// "pending sync" for queued requests.
    public protocol WatchQueueTransport: Sendable {
        func enqueue(_ envelope: WatchRelayEnvelope) async
    }

    /// Watch-side gateway **routing engine** (docs/MOBILE_APP.md watch section;
    /// D-K4).
    ///
    /// Route selection is relay-first (WatchConnectivity through the phone —
    /// there is no Tailscale on watchOS), then direct HTTP when the gateway is
    /// reachable, else queue. An interactive request times out per route and
    /// falls through (relay → direct → queue) inside the < 5s approval budget.
    ///
    /// This is the low-level engine over the three transports; the watch app's
    /// UI-facing `WatchGatewayProxy` protocol (in the watch target) wraps a
    /// router to serve the SwiftUI surface. Kept a distinct type so the routing
    /// policy is unit-tested here in isolation from the UI seam.
    public struct WatchGatewayRouter: Sendable {
        private let relay: any WatchRelayTransport
        private let direct: any WatchDirectTransport
        private let queue: any WatchQueueTransport
        /// The watch's own token, embedded in a relayed envelope so the phone
        /// forwards it opaquely; `nil` when unprovisioned (relay fails closed).
        private let watchToken: String?

        public init(
            relay: any WatchRelayTransport,
            direct: any WatchDirectTransport,
            queue: any WatchQueueTransport,
            watchToken: String?
        ) {
            self.relay = relay
            self.direct = direct
            self.queue = queue
            self.watchToken = watchToken
        }

        private var reachability: WatchReachability {
            WatchReachability(
                relayReachable: relay.isReachable && watchToken != nil,
                directReachable: direct.isReachable)
        }

        /// Approve/deny a pending tool call. Low-risk only from the watch — the
        /// caller (watch UI) refuses to build a high-risk approve (D-K3/D-K4).
        @discardableResult
        public func approve(
            requestID: String, threadID: String?, action: String
        ) async -> WatchRouteOutcome {
            await send(.approve(requestID: requestID, threadID: threadID, action: action))
        }

        /// Send a dictated quick prompt.
        @discardableResult
        public func quickAsk(_ prompt: String, threadID: String?) async -> WatchRouteOutcome {
            await send(.quickAsk(prompt: prompt, threadID: threadID))
        }

        /// Ask for a fresh snapshot. Relay/direct only — there is nothing to
        /// queue for a read that a later push supersedes, so an unreachable
        /// gateway simply yields `.pendingSync` without enqueuing.
        @discardableResult
        public func refreshSnapshot() async -> WatchRouteOutcome {
            await send(.snapshotRefresh, queueable: false)
        }

        // MARK: - Routing core

        private func send(
            _ request: WatchRelayRequest,
            queueable: Bool = true
        ) async -> WatchRouteOutcome {
            let reachability = self.reachability
            var route = WatchRouteSelector.primaryRoute(for: reachability)

            while route != .queued {
                do {
                    let response = try await attempt(route, request: request)
                    return .completed(route: route, response: response)
                } catch {
                    // Timeout / transport failure: fall through to the next live
                    // route, or drop to the queue when none remain.
                    guard
                        let next = WatchRouteSelector.nextRoute(
                            after: route, reachability: reachability)
                    else { break }
                    route = next
                }
            }

            // No live route completed. Queue when the request is queueable and we
            // actually hold a token to relay later; otherwise report pending sync.
            if queueable, let envelope = relayEnvelope(for: request) {
                await queue.enqueue(envelope)
                return .queued
            }
            return .pendingSync
        }

        // Attempt one live route. `.queued` is never passed here (the loop
        // guards it out); it is handled as `noRouteAvailable` for safety.
        private func attempt(
            _ route: WatchRoute, request: WatchRelayRequest
        ) async throws -> WatchRelayResponse {
            switch route {
            case .relay: return try await relay(request)
            case .direct: return try await direct.direct(request)
            case .queued: throw WatchRelayError.noRouteAvailable
            }
        }

        private func relay(_ request: WatchRelayRequest) async throws -> WatchRelayResponse {
            guard let envelope = relayEnvelope(for: request) else {
                // Unprovisioned: cannot attribute to the watch. Treat as a route
                // failure so we fall through to direct/queue.
                throw WatchRelayError.noRouteAvailable
            }
            return try await relay.relay(envelope)
        }

        private func relayEnvelope(for request: WatchRelayRequest) -> WatchRelayEnvelope? {
            guard let watchToken else { return nil }
            return WatchRelayEnvelope(watchToken: watchToken, request: request)
        }
    }

    /// The result of a watch RPC after route selection + fall-through.
    public enum WatchRouteOutcome: Sendable, Equatable {
        /// A live route (relay or direct) returned a gateway outcome.
        case completed(route: WatchRoute, response: WatchRelayResponse)
        /// No live route; the request was persisted for later delivery.
        case queued
        /// No live route and nothing to queue (e.g. a snapshot read); the UI
        /// shows "pending sync".
        case pendingSync
    }
#endif
