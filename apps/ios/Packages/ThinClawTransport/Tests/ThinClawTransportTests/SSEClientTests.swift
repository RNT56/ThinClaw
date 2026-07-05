import Foundation
import Testing

@testable import ThinClawTransport

@Suite("SSEClient")
struct SSEClientTests {
    @Test("streams parsed events from an injected byte sequence")
    func streamsParsedEvents() async throws {
        let client = SSEClient()
        let input = ScriptedByteStream(
            text: "data: one\n\nevent: custom\ndata: two\n\n", chunkSize: 3)

        var received: [ServerSentEvent] = []
        for try await event in await client.events(from: input) {
            received.append(event)
        }
        #expect(
            received == [
                ServerSentEvent(data: "one"),
                ServerSentEvent(event: "custom", data: "two"),
            ])
    }

    @Test("finishes cleanly when the byte stream ends mid-event")
    func discardsTrailingPartialEvent() async throws {
        let client = SSEClient()
        let input = ScriptedByteStream(text: "data: complete\n\ndata: dangling")

        var received: [ServerSentEvent] = []
        for try await event in await client.events(from: input) {
            received.append(event)
        }
        #expect(received == [ServerSentEvent(data: "complete")])
    }

    @Test("rethrows transport errors after yielding prior events")
    func propagatesErrors() async {
        let client = SSEClient()
        let input = ScriptedByteStream(
            text: "data: before failure\n\n",
            finalError: ScriptedStreamError())

        var received: [ServerSentEvent] = []
        do {
            for try await event in await client.events(from: input) {
                received.append(event)
            }
            Issue.record("expected the stream to throw")
        } catch {
            #expect(error is ScriptedStreamError)
        }
        #expect(received == [ServerSentEvent(data: "before failure")])
    }

    @Test("exposes lastEventID and retry for the reconnect layer")
    func exposesStreamState() async throws {
        let client = SSEClient()
        let input = ScriptedByteStream(text: "retry: 2500\nid: evt-9\ndata: x\n\n")

        for try await _ in await client.events(from: input) {}

        #expect(await client.lastEventID == "evt-9")
        #expect(await client.reconnectionTime == .milliseconds(2500))
    }

    @Test("lastEventID is nil before any id field is seen")
    func lastEventIDInitiallyNil() async {
        let client = SSEClient()
        #expect(await client.lastEventID == nil)
        #expect(await client.reconnectionTime == nil)
    }
}
