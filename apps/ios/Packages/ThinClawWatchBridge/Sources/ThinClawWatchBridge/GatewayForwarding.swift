#if canImport(Security) && canImport(CryptoKit)
    import Foundation
    import ThinClawAPI
    import ThinClawAuth

    /// The gateway calls the watch bridge makes, abstracted so both the host
    /// (relay) side and the companion-provisioning flow share one pinned-client
    /// assembly, and so tests can inject a stub without a live gateway.
    ///
    /// Every call is built over a **pinned** `URLSession` (SPKI pin from the
    /// credential) so bridge traffic can never bypass TLS pinning / the D-X2
    /// policy — the same seam the app, NSE, and widgets use
    /// (docs/MOBILE_SECURITY.md).
    public protocol WatchBridgeGateway: Sendable {
        /// Mint a reduced-scope companion for the paired watch
        /// (`POST /api/devices/me/companions`, `devices:self`). The parent
        /// credential authenticates; the returned token is the watch's own.
        func mintCompanion(name: String) async throws -> CreatedCompanion
        /// Revoke a companion by id (`DELETE /api/devices/me/companions/{id}`).
        func revokeCompanion(deviceID: String) async throws
        /// Relay an approval decision **with the watch's own token** forwarded
        /// opaquely (`POST /api/chat/approval`). The phone never substitutes its
        /// own credential (D-K4).
        func forwardApproval(
            watchToken: String, requestID: String, threadID: String?, action: String
        ) async throws
        /// Relay a quick-ask prompt with the watch's own token
        /// (`POST /api/chat/send`); returns the gateway `message_id`.
        func forwardQuickAsk(
            watchToken: String, prompt: String, threadID: String?
        ) async throws -> String
    }

    /// The minted companion, surfaced to the provisioning flow.
    public struct CreatedCompanion: Sendable, Equatable {
        public var deviceID: String
        public var parentDeviceID: String
        public var token: String

        public init(deviceID: String, parentDeviceID: String, token: String) {
            self.deviceID = deviceID
            self.parentDeviceID = parentDeviceID
            self.token = token
        }
    }

    /// Production ``WatchBridgeGateway`` over the pinned generated client.
    ///
    /// Built from the phone's own ``DeviceCredential`` — the parent that owns
    /// the `devices:self` scope needed to mint/revoke companions. The relayed
    /// approval/quick-ask calls, by contrast, authenticate with the **watch's**
    /// token passed per-call: the phone assembles a client whose bearer is the
    /// forwarded watch token, so the gateway attributes the request to the watch
    /// and can revoke it independently (D-K4).
    public struct LiveWatchBridgeGateway: WatchBridgeGateway {
        private let parentCredential: DeviceCredential

        public init(parentCredential: DeviceCredential) {
            self.parentCredential = parentCredential
        }

        // A pinned client authenticating as `token` against the parent's
        // policy-allowed base URL. `token == nil` ⇒ the parent's own token.
        private func client(bearer token: String?) throws -> Client {
            guard let baseURL = parentCredential.preferredBaseURL else {
                throw WatchBridgeGatewayError.noReachableGateway
            }
            let bearer = token ?? parentCredential.deviceToken
            let session = SharedGatewayConnection.pinnedSession(for: parentCredential)
            return GatewayClient.make(baseURL: baseURL, token: { bearer }, session: session)
        }

        public func mintCompanion(name: String) async throws -> CreatedCompanion {
            let client = try client(bearer: nil)
            do {
                let output = try await client.devicesMeCompanionsCreateHandler(
                    body: .json(.init(name: name, platform: "watchos")))
                let json = try output.ok.body.json
                return CreatedCompanion(
                    deviceID: json.deviceId,
                    parentDeviceID: json.parentDeviceId,
                    token: json.token)
            } catch {
                throw WatchBridgeGatewayError.gateway(APIError.from(error))
            }
        }

        public func revokeCompanion(deviceID: String) async throws {
            let client = try client(bearer: nil)
            do {
                _ = try await client.devicesMeCompanionsRevokeHandler(
                    path: .init(id: deviceID))
            } catch {
                throw WatchBridgeGatewayError.gateway(APIError.from(error))
            }
        }

        public func forwardApproval(
            watchToken: String, requestID: String, threadID: String?, action: String
        ) async throws {
            // The bearer is the WATCH token, forwarded opaquely (D-K4).
            let client = try client(bearer: watchToken)
            do {
                _ = try await client.chatApprovalHandler(
                    body: .json(
                        .init(
                            action: action, requestId: requestID, threadId: threadID)))
            } catch {
                throw WatchBridgeGatewayError.gateway(APIError.from(error))
            }
        }

        public func forwardQuickAsk(
            watchToken: String, prompt: String, threadID: String?
        ) async throws -> String {
            let client = try client(bearer: watchToken)
            do {
                let output = try await client.chatSendHandler(
                    body: .json(.init(content: prompt, threadId: threadID)))
                return try output.accepted.body.json.messageId
            } catch {
                throw WatchBridgeGatewayError.gateway(APIError.from(error))
            }
        }
    }

    public enum WatchBridgeGatewayError: Error, Sendable {
        case noReachableGateway
        case gateway(APIError)

        /// Whether this error means the watch's companion token is no longer
        /// valid (401/403 from a forwarded call) — the phone should re-provision.
        public var indicatesRevokedCompanion: Bool {
            if case .gateway(let apiError) = self {
                switch apiError {
                case .unauthorized, .forbidden: return true
                default: return false
                }
            }
            return false
        }
    }
#endif
