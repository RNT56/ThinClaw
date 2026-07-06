import Foundation
import Testing

@testable import ThinClawWatchBridge

@Suite("WatchRelayEnvelope wire coding")
struct WatchRelayEnvelopeTests {
    @Test("A relayed approval carries the WATCH token, not the phone's")
    func relayedApprovalCarriesWatchToken() throws {
        // The phone forwards this envelope opaquely; the token inside is what
        // authenticates at the gateway (D-K4). The invariant under test: the
        // watch token — and only the watch token — rides in the envelope.
        let watchToken = "tcd_watch_companion_abc"
        let envelope = WatchRelayEnvelope(
            watchToken: watchToken,
            request: .approve(requestID: "req-1", threadID: "thr-1", action: "approve"))

        let roundTripped = try WatchRelayEnvelope.decode(envelope.encoded())

        #expect(roundTripped.watchToken == watchToken)
        #expect(
            roundTripped.request
                == .approve(
                    requestID: "req-1", threadID: "thr-1", action: "approve"))
    }

    @Test("Every request case round-trips through the envelope")
    func allRequestsRoundTrip() throws {
        let cases: [WatchRelayRequest] = [
            .approve(requestID: "r", threadID: nil, action: "deny"),
            .quickAsk(prompt: "what's my schedule", threadID: "t"),
            .snapshotRefresh,
        ]
        for request in cases {
            let envelope = WatchRelayEnvelope(watchToken: "tcd_x", request: request)
            let back = try WatchRelayEnvelope.decode(envelope.encoded())
            #expect(back.request == request)
        }
    }

    @Test("Direct-route envelope omits the token from the wire")
    func directEnvelopeOmitsToken() throws {
        // On the direct path the watch signs the request itself, so the token
        // never needs to leave the keychain over the wire.
        let envelope = WatchRelayEnvelope(
            watchToken: nil, request: .snapshotRefresh)
        let back = try WatchRelayEnvelope.decode(envelope.encoded())
        #expect(back.watchToken == nil)
    }

    @Test("An unknown envelope version is rejected (fail-closed)")
    func unknownVersionRejected() throws {
        var envelope = WatchRelayEnvelope(watchToken: "tcd_x", request: .snapshotRefresh)
        envelope.version = 999
        let data = try envelope.encoded()
        #expect(throws: WatchRelayError.unsupportedVersion(999)) {
            try WatchRelayEnvelope.decode(data)
        }
    }

    @Test("sendMessage payload round-trips via the message key")
    func messagePayloadRoundTrips() throws {
        let envelope = WatchRelayEnvelope(
            watchToken: "tcd_x",
            request: .quickAsk(prompt: "hi", threadID: nil))
        let message = try envelope.messagePayload()
        #expect(message[WatchRelayEnvelope.messageKey] != nil)
        let back = try WatchRelayEnvelope.fromMessage(message)
        #expect(back == envelope)
    }

    @Test("A message without the envelope key is malformed")
    func malformedMessageRejected() {
        #expect(throws: WatchRelayError.malformedMessage) {
            try WatchRelayEnvelope.fromMessage(["nonsense": 1])
        }
    }

    @Test("Response accepted encodes the quick-ask message id")
    func responseRoundTrips() throws {
        let responses: [WatchRelayResponse] = [
            .accepted,
            .accepted(messageID: "msg-9"),
            .failed(reason: "429"),
            .reprovisionRequired,
        ]
        for response in responses {
            let data = try JSONEncoder().encode(response)
            let back = try JSONDecoder().decode(WatchRelayResponse.self, from: data)
            #expect(back == response)
        }
    }
}
