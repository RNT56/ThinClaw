#if canImport(Security) && canImport(CryptoKit)
    import Foundation
    import Testing

    @testable import ThinClawWatchBridge

    /// Records the bearer token each forwarded call authenticated with, so the
    /// test can assert the **watch** token — never a phone token — rides in a
    /// relayed approval (D-K4). This stub stands in for the live gateway.
    private final class RecordingGateway: WatchBridgeGateway, @unchecked Sendable {
        var mintName: String?
        var mintResult = CreatedCompanion(
            deviceID: "dev-watch-1", parentDeviceID: "dev-phone-1",
            token: "tcd_watch_minted")
        var revokedID: String?
        var approvalBearer: String?
        var approvalRequestID: String?
        var approvalAction: String?
        var quickAskBearer: String?

        func mintCompanion(name: String) async throws -> CreatedCompanion {
            mintName = name
            return mintResult
        }
        func revokeCompanion(deviceID: String) async throws { revokedID = deviceID }
        func forwardApproval(
            watchToken: String, requestID: String, threadID: String?, action: String
        ) async throws {
            approvalBearer = watchToken
            approvalRequestID = requestID
            approvalAction = action
        }
        func forwardQuickAsk(
            watchToken: String, prompt: String, threadID: String?
        ) async throws -> String {
            quickAskBearer = watchToken
            return "msg-1"
        }
    }

    @Suite("Relay forwarding attributes to the watch token")
    struct RelayForwardingTests {
        @Test("A relayed approval authenticates with the watch token, not the phone's")
        func approvalUsesWatchToken() async throws {
            let gateway = RecordingGateway()
            let phoneToken = "tcd_phone_parent"
            let watchToken = "tcd_watch_companion"

            let responder = WatchRelayResponder(gateway: gateway)
            let response = await responder.answer(
                WatchRelayEnvelope(
                    watchToken: watchToken,
                    request: .approve(requestID: "r-9", threadID: "t-1", action: "approve")),
                phoneToken: phoneToken)

            #expect(gateway.approvalBearer == watchToken)
            #expect(gateway.approvalBearer != phoneToken)
            #expect(gateway.approvalRequestID == "r-9")
            #expect(gateway.approvalAction == "approve")
            #expect(response == .accepted)
        }

        @Test("A relay envelope with no watch token cannot forward — fail-closed")
        func missingWatchTokenReprovisions() async throws {
            let gateway = RecordingGateway()
            let responder = WatchRelayResponder(gateway: gateway)
            let response = await responder.answer(
                WatchRelayEnvelope(
                    watchToken: nil,
                    request: .approve(requestID: "r", threadID: nil, action: "approve")),
                phoneToken: "tcd_phone")

            #expect(gateway.approvalBearer == nil)  // never forwarded
            #expect(response == .reprovisionRequired)
        }

        @Test("A quick-ask relays the watch token and echoes the message id")
        func quickAskUsesWatchTokenAndEchoesID() async throws {
            let gateway = RecordingGateway()
            let responder = WatchRelayResponder(gateway: gateway)
            let response = await responder.answer(
                WatchRelayEnvelope(
                    watchToken: "tcd_watch",
                    request: .quickAsk(prompt: "hi", threadID: nil)),
                phoneToken: "tcd_phone")

            #expect(gateway.quickAskBearer == "tcd_watch")
            #expect(response == .accepted(messageID: "msg-1"))
        }
    }
#endif
