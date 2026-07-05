import Foundation
import Testing

@testable import ThinClawTransport

@Suite("SSEParser field handling")
struct SSEParserFieldTests {
    @Test("single data line dispatches on blank line")
    func singleDataLine() {
        var parser = SSEParser()
        let events = parser.feed(Array("data: hello\n\n".utf8))
        #expect(events == [ServerSentEvent(event: "message", data: "hello")])
    }

    @Test("nothing dispatches before the blank line")
    func noDispatchWithoutBlankLine() {
        var parser = SSEParser()
        #expect(parser.feed(Array("data: hello\n".utf8)).isEmpty)
        #expect(parser.feed(Array("\n".utf8)).count == 1)
    }

    @Test("multi-line data joins with newline")
    func multiLineData() {
        var parser = SSEParser()
        let events = parser.feed(Array("data: line one\ndata: line two\n\n".utf8))
        #expect(events.map(\.data) == ["line one\nline two"])
    }

    @Test("value keeps only ONE leading space stripped")
    func leadingSpaceStripping() {
        var parser = SSEParser()
        let events = parser.feed(Array("data:  double spaced\ndata:tight\n\n".utf8))
        #expect(events.map(\.data) == [" double spaced\ntight"])
    }

    @Test("event field sets the type; type resets after dispatch")
    func eventTypeField() {
        var parser = SSEParser()
        var events = parser.feed(Array("event: custom\ndata: a\n\n".utf8))
        events += parser.feed(Array("data: b\n\n".utf8))
        #expect(events.map(\.event) == ["custom", "message"])
    }

    @Test("comment lines are ignored")
    func commentLines() {
        var parser = SSEParser()
        let events = parser.feed(Array(": ping\n: another comment\ndata: real\n\n".utf8))
        #expect(events == [ServerSentEvent(data: "real")])
    }

    @Test("comment-only block dispatches nothing")
    func commentOnlyBlock() {
        var parser = SSEParser()
        #expect(parser.feed(Array(": keep-alive\n\n".utf8)).isEmpty)
    }

    @Test("event field without data dispatches nothing but resets type")
    func eventWithoutData() {
        var parser = SSEParser()
        #expect(parser.feed(Array("event: orphan\n\n".utf8)).isEmpty)
        let events = parser.feed(Array("data: next\n\n".utf8))
        #expect(events.map(\.event) == ["message"], "orphan type must not leak")
    }

    @Test("field line without colon is a field with empty value")
    func lineWithoutColon() {
        var parser = SSEParser()
        // Bare "data" contributes an empty data line -> dispatches "".
        let events = parser.feed(Array("data\n\n".utf8))
        #expect(events.map(\.data) == [""])
    }

    @Test("unknown fields are ignored")
    func unknownFields() {
        var parser = SSEParser()
        let events = parser.feed(Array("frobnicate: 12\ndata: kept\nx: y\n\n".utf8))
        #expect(events == [ServerSentEvent(data: "kept")])
    }

    @Test("id field is remembered and stamped onto later events")
    func idField() {
        var parser = SSEParser()
        var events = parser.feed(Array("id: 41\ndata: a\n\n".utf8))
        events += parser.feed(Array("data: b\n\n".utf8))  // id persists
        events += parser.feed(Array("id: 42\ndata: c\n\n".utf8))
        #expect(events.map(\.lastEventID) == ["41", "41", "42"])
        #expect(parser.lastEventID == "42")
    }

    @Test("id containing NUL is ignored per spec")
    func idWithNULIgnored() {
        var parser = SSEParser()
        var bytes = Array("id: a".utf8)
        bytes.append(0)
        bytes.append(contentsOf: Array("b\ndata: x\n\n".utf8))
        let events = parser.feed(bytes)
        #expect(events.map(\.lastEventID) == [nil])
    }

    @Test("valid retry field sets reconnection time")
    func retryField() {
        var parser = SSEParser()
        _ = parser.feed(Array("retry: 5000\n\n".utf8))
        #expect(parser.reconnectionTime == .milliseconds(5000))
    }

    @Test("non-numeric retry is ignored")
    func invalidRetryIgnored() {
        var parser = SSEParser()
        _ = parser.feed(Array("retry: 5s\nretry: -3\nretry:\n\n".utf8))
        #expect(parser.reconnectionTime == nil)
    }

    @Test("leading UTF-8 BOM is stripped once")
    func bomStripped() {
        var parser = SSEParser()
        var bytes: [UInt8] = [0xEF, 0xBB, 0xBF]
        bytes.append(contentsOf: Array("data: after bom\n\n".utf8))
        let events = parser.feed(bytes)
        #expect(events.map(\.data) == ["after bom"])
    }

    @Test("empty data field value still dispatches an empty event")
    func emptyDataDispatches() {
        var parser = SSEParser()
        let events = parser.feed(Array("data:\n\n".utf8))
        #expect(events.map(\.data) == [""])
    }
}

@Suite("SSEParser line endings")
struct SSEParserLineEndingTests {
    @Test("CRLF line endings parse identically to LF")
    func crlfEquivalence() {
        var parser = SSEParser()
        let events = parser.feed(Array("data: one\r\ndata: two\r\n\r\n".utf8))
        #expect(events.map(\.data) == ["one\ntwo"])
    }

    @Test("bare CR line endings parse identically to LF")
    func bareCR() {
        var parser = SSEParser()
        let events = parser.feed(Array("data: one\rdata: two\r\r".utf8))
        #expect(events.map(\.data) == ["one\ntwo"])
    }

    @Test("CRLF split across two chunks is one line ending, not two")
    func crlfSplitAcrossChunks() {
        var parser = SSEParser()
        var events = parser.feed(Array("data: a\r".utf8))
        events += parser.feed(Array("\ndata: b\r".utf8))
        events += parser.feed(Array("\n\r".utf8))
        events += parser.feed(Array("\n".utf8))
        #expect(events.map(\.data) == ["a\nb"])
    }

    @Test("CR followed by non-LF byte does not swallow the byte")
    func crFollowedByContent() {
        var parser = SSEParser()
        let events = parser.feed(Array("data: a\rdata: b\n\n".utf8))
        #expect(events.map(\.data) == ["a\nb"])
    }
}

@Suite("SSEParser incremental buffering")
struct SSEParserBufferingTests {
    @Test("incomplete trailing line is retained across feeds")
    func trailingBufferRetention() {
        var parser = SSEParser()
        #expect(parser.feed(Array("data: par".utf8)).isEmpty)
        #expect(parser.feed(Array("tial".utf8)).isEmpty)
        let events = parser.feed(Array("ly split\n\n".utf8))
        #expect(events.map(\.data) == ["partially split"])
    }

    @Test("UTF-8 multibyte scalar split across chunks decodes intact")
    func utf8SplitAcrossChunks() {
        let text = "data: caffè 🦀 done\n\n"
        let bytes = Array(text.utf8)
        // Split inside the 4-byte crab scalar. Find its offset first.
        let crabStart = bytes.firstRange(of: Array("🦀".utf8))!.lowerBound
        var parser = SSEParser()
        var events = parser.feed(bytes[..<(crabStart + 2)])
        events += parser.feed(bytes[(crabStart + 2)...])
        #expect(events.map(\.data) == ["caffè 🦀 done"])
    }

    @Test("byte-by-byte feeding yields identical results")
    func byteByByte() {
        let bytes = Array("event: e\nid: 7\ndata: x\ndata: y\n\ndata: z\n\n".utf8)
        let wholesale = parseChunked(bytes, size: .max)
        let trickled = parseChunked(bytes, size: 1)
        #expect(wholesale == trickled)
        #expect(trickled.count == 2)
    }

    @Test("finish() discards an un-terminated trailing event per spec")
    func finishDiscardsIncompleteEvent() {
        var parser = SSEParser()
        #expect(parser.feed(Array("data: never terminated\n".utf8)).isEmpty)
        parser.finish()
        // A fresh event afterwards must not inherit stale data.
        let events = parser.feed(Array("data: clean\n\n".utf8))
        #expect(events.map(\.data) == ["clean"])
    }
}
