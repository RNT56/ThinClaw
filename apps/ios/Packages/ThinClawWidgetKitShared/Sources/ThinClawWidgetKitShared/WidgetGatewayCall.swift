#if canImport(Security) && canImport(CryptoKit)
    import Foundation
    import ThinClawAPI
    import ThinClawAuth

    /// The gateway REST plumbing the interactive widget intents use.
    ///
    /// The widget runs in a short-lived extension process. It reads the device
    /// token from the **shared Keychain** and connects over a **pinned**
    /// `URLSession` assembled by ``SharedGatewayConnection`` — the same seam the
    /// app and NSE use — so widget traffic can never bypass TLS pinning or the
    /// D-X2 `ConnectionPolicy` (docs/MOBILE_SECURITY.md). There is deliberately
    /// no unpinned code path here.
    public enum WidgetGatewayCall {
        public enum Failure: Error, Equatable {
            /// No device credential in the shared Keychain (not paired).
            case notPaired
            /// The credential has no policy-allowed base URL to reach.
            case noReachableGateway
            /// The gateway rejected or failed the request.
            case gateway(String)
        }

        /// Submit an approval decision for `requestID`. `action` is the raw
        /// gateway verb (`"approve"` or `"deny"`).
        ///
        /// Idempotent server-side by `request_id`, so a retry after an
        /// unreachable-gateway failure is safe. High-risk gating is the
        /// **caller's** responsibility — this helper does not classify risk;
        /// the intents refuse to build a high-risk approve before reaching
        /// here (D-K3).
        public static func submitApproval(
            requestID: String,
            threadID: String?,
            action: String
        ) async throws {
            guard let credential = SharedGatewayConnection.loadCredential() else {
                throw Failure.notPaired
            }
            guard let baseURL = credential.preferredBaseURL else {
                throw Failure.noReachableGateway
            }

            let token = credential.deviceToken
            let session = SharedGatewayConnection.pinnedSession(for: credential)
            let client = GatewayClient.make(
                baseURL: baseURL,
                token: { token },
                session: session
            )

            do {
                _ = try await client.chatApprovalHandler(
                    body: .json(
                        .init(
                            action: action,
                            requestId: requestID,
                            threadId: threadID
                        )))
            } catch {
                throw Failure.gateway(String(describing: error))
            }
        }

        /// Send a Quick Ask prompt. The reply is delivered as a push when this
        /// device holds no live stream (docs/MOBILE_APP.md). Returns the
        /// gateway `message_id` on acceptance.
        @discardableResult
        public static func sendPrompt(
            _ content: String,
            threadID: String?
        ) async throws -> String {
            guard let credential = SharedGatewayConnection.loadCredential() else {
                throw Failure.notPaired
            }
            guard let baseURL = credential.preferredBaseURL else {
                throw Failure.noReachableGateway
            }

            let token = credential.deviceToken
            let session = SharedGatewayConnection.pinnedSession(for: credential)
            let client = GatewayClient.make(
                baseURL: baseURL,
                token: { token },
                session: session
            )

            do {
                let output = try await client.chatSendHandler(
                    body: .json(.init(content: content, threadId: threadID)))
                return try output.accepted.body.json.messageId
            } catch {
                throw Failure.gateway(String(describing: error))
            }
        }
    }
#endif
