#if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
    import Foundation
    import ThinClawAuth
    import ThinClawWatchBridge
    import WatchConnectivity

    /// The live WatchConnectivity relay transport (docs/MOBILE_SECURITY.md D-K4).
    ///
    /// Sends an envelope (carrying the watch's own token) to the phone over
    /// `WCSession.sendMessage` and awaits the phone's forwarded gateway outcome.
    /// Reachability is "activated + phone reachable" — only then can an
    /// interactive `sendMessage` reply arrive within the approval budget.
    // `@unchecked Sendable`: the only stored reference is a `WCSession`, whose
    // `sendMessage`/`activationState`/`isReachable` are documented thread-safe;
    // the struct adds no mutable shared state of its own.
    struct LiveWatchRelayTransport: WatchRelayTransport, @unchecked Sendable {
        let session: WCSession
        /// Per-attempt deadline; the router falls through to direct/queue on a
        /// timeout inside the < 5s approval budget.
        var deadline: TimeInterval = WatchRelayTiming.routeDeadline

        var isReachable: Bool {
            session.activationState == .activated && session.isReachable
        }

        func relay(_ envelope: WatchRelayEnvelope) async throws -> WatchRelayResponse {
            return try await withThrowingTaskGroup(of: WatchRelayResponse.self) { group in
                group.addTask {
                    let payload = try envelope.messagePayload()
                    return try await Self.sendMessage(session, payload)
                }
                group.addTask {
                    try await Task.sleep(nanoseconds: UInt64(deadline * 1_000_000_000))
                    throw WatchRelayError.timedOut
                }
                defer { group.cancelAll() }
                guard let result = try await group.next() else {
                    throw WatchRelayError.timedOut
                }
                return result
            }
        }

        /// Bridge `sendMessage`'s reply/error handlers into an async result.
        private static func sendMessage(
            _ session: WCSession,
            _ payload: [String: Any]
        ) async throws -> WatchRelayResponse {
            try await withCheckedThrowingContinuation { continuation in
                session.sendMessage(
                    payload,
                    replyHandler: { reply in
                        if let response = try? WatchRelayResponse.fromMessage(reply) {
                            continuation.resume(returning: response)
                        } else {
                            continuation.resume(
                                returning: .failed(reason: "malformed"))
                        }
                    },
                    errorHandler: { _ in
                        continuation.resume(throwing: WatchRelayError.noRouteAvailable)
                    })
            }
        }
    }

    /// The direct-to-gateway transport: the watch signs the request itself with
    /// its own credential over a **pinned** URLSession (D-X2), reusing the bridge's
    /// ``LiveWatchBridgeGateway`` client assembly. No token rides on the wire — it
    /// authenticates as a bearer header built from the watch's own credential.
    ///
    /// "Reachable" means the watch holds a companion credential with a
    /// policy-allowed direct base URL. In practice the watch has no tailnet, so
    /// this is a pinned LAN / public-HTTPS fallback used only when relay is down.
    struct LiveWatchDirectTransport: WatchDirectTransport {
        /// The watch's own credential, or `nil` when unprovisioned.
        let credential: WatchCompanionCredential?

        var isReachable: Bool {
            credential?.deviceCredential.preferredBaseURL != nil
        }

        func direct(_ request: WatchRelayRequest) async throws -> WatchRelayResponse {
            guard let credential else { throw WatchRelayError.noRouteAvailable }
            // The watch's own credential is the "parent" of its own direct calls:
            // its `deviceToken` is the watch token, so the pinned client
            // authenticates the request as the watch (D-K4/D-X2).
            let deviceCredential = credential.deviceCredential
            let gateway = LiveWatchBridgeGateway(parentCredential: deviceCredential)
            let watchToken = credential.watchToken
            do {
                switch request {
                case let .approve(requestID, threadID, action):
                    try await gateway.forwardApproval(
                        watchToken: watchToken, requestID: requestID,
                        threadID: threadID, action: action)
                    return .accepted
                case let .quickAsk(prompt, threadID):
                    let messageID = try await gateway.forwardQuickAsk(
                        watchToken: watchToken, prompt: prompt, threadID: threadID)
                    return .accepted(messageID: messageID)
                case .snapshotRefresh:
                    // A direct snapshot pull is not modelled as a gateway call
                    // here; the phone's mirror is the snapshot source. Report
                    // acceptance so the router does not fall through pointlessly.
                    return .accepted
                }
            } catch let error as WatchBridgeGatewayError {
                if error.indicatesRevokedCompanion { return .reprovisionRequired }
                throw error
            }
        }
    }

    /// The offline queue: persist an envelope for delivery when a live route
    /// returns, via `WCSession.transferUserInfo` (the OS retries delivery). The
    /// phone answers a queued RPC from its `didReceiveUserInfo` handler.
    // `@unchecked Sendable`: `transferUserInfo` is thread-safe on `WCSession`;
    // no other shared mutable state.
    struct LiveWatchQueueTransport: WatchQueueTransport, @unchecked Sendable {
        let session: WCSession

        func enqueue(_ envelope: WatchRelayEnvelope) async {
            guard let payload = try? envelope.messagePayload() else { return }
            session.transferUserInfo(payload)
        }
    }
#endif
