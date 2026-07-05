import Foundation

/// Incremental, allocation-conscious parser for the Server-Sent Events subset
/// the ThinClaw gateway emits.
///
/// Implements the WHATWG event-stream processing model for:
/// - `data:` fields (multiple lines joined with `\n`),
/// - `event:` fields,
/// - `id:` fields (ignored when they contain U+0000, per spec),
/// - `retry:` fields (ASCII-digits-only, otherwise ignored),
/// - comment lines (leading `:`),
/// - unknown field names (ignored),
/// - dispatch on blank line, with no dispatch when no `data:` field was seen,
/// - LF, CRLF, and bare-CR line endings — including a CR/LF pair split
///   across two chunks,
/// - an optional UTF-8 BOM at the very start of the stream.
///
/// Feed it arbitrary byte chunks as they arrive; it buffers the incomplete
/// trailing line between calls, so UTF-8 sequences split across chunk
/// boundaries are never mis-decoded (lines are only decoded once complete,
/// and `\n` / `\r` are single-byte and cannot occur inside a multi-byte
/// UTF-8 scalar).
///
/// This is a plain value type: no locks, no I/O. ``SSEClient`` owns one and
/// provides the async plumbing.
public struct SSEParser: Sendable {
    /// Bytes of the current, not-yet-terminated line.
    private var lineBuffer: [UInt8] = []
    /// Accumulated `data:` field values for the in-flight event.
    private var dataLines: [String] = []
    /// Whether any `data:` field was seen for the in-flight event
    /// (distinguishes "no data" from a single empty `data:` line).
    private var sawData = false
    /// Value of the `event:` field for the in-flight event.
    private var eventType = ""
    /// Stream-level "last event ID", updated by `id:` fields.
    public private(set) var lastEventID = ""
    /// Reconnection delay requested by the most recent valid `retry:` field.
    public private(set) var reconnectionTime: Duration?

    private var pendingCR = false
    private var atStreamStart = true

    public init() {}

    /// Feed a chunk of bytes; returns every event that became complete.
    public mutating func feed(_ bytes: some Sequence<UInt8>) -> [ServerSentEvent] {
        var dispatched: [ServerSentEvent] = []
        for byte in bytes {
            if pendingCR {
                pendingCR = false
                if byte == 0x0A {
                    // LF completing a CRLF whose CR already ended the line
                    // (possibly in a previous chunk) — swallow it.
                    continue
                }
            }
            switch byte {
            case 0x0A:  // LF
                if let event = consumeLine() { dispatched.append(event) }
            case 0x0D:  // CR terminates the line; a following LF is swallowed.
                pendingCR = true
                if let event = consumeLine() { dispatched.append(event) }
            default:
                lineBuffer.append(byte)
            }
        }
        return dispatched
    }

    /// Signal end-of-stream. Per spec, an event that was not terminated by a
    /// blank line is discarded, as is an incomplete trailing line.
    public mutating func finish() {
        lineBuffer.removeAll()
        dataLines.removeAll()
        sawData = false
        eventType = ""
        pendingCR = false
    }

    // MARK: - Line handling

    private mutating func consumeLine() -> ServerSentEvent? {
        var bytes = lineBuffer[...]
        if atStreamStart {
            atStreamStart = false
            // One-time UTF-8 BOM strip at the very start of the stream.
            if bytes.starts(with: [0xEF, 0xBB, 0xBF]) {
                bytes = bytes.dropFirst(3)
            }
        }
        defer { lineBuffer.removeAll(keepingCapacity: true) }

        if bytes.isEmpty {
            return dispatchEvent()
        }
        if bytes.first == UInt8(ascii: ":") {
            return nil  // comment line
        }

        let name: Substring
        var value: Substring
        let line = String(decoding: bytes, as: UTF8.self)
        if let colon = line.firstIndex(of: ":") {
            name = line[..<colon]
            value = line[line.index(after: colon)...]
            if value.first == " " { value = value.dropFirst() }
        } else {
            // A line with no colon is a field with an empty value.
            name = line[...]
            value = ""
        }
        processField(name: name, value: value)
        return nil
    }

    private mutating func processField(name: Substring, value: Substring) {
        switch name {
        case "data":
            dataLines.append(String(value))
            sawData = true
        case "event":
            eventType = String(value)
        case "id":
            // Per spec: ignore ids containing U+0000 NULL.
            if !value.utf8.contains(0) {
                lastEventID = String(value)
            }
        case "retry":
            if !value.isEmpty,
                value.utf8.allSatisfy({ (0x30...0x39).contains($0) }),
                let milliseconds = Int64(value)
            {
                reconnectionTime = .milliseconds(milliseconds)
            }
        default:
            break  // unknown field: ignored per spec
        }
    }

    private mutating func dispatchEvent() -> ServerSentEvent? {
        defer {
            dataLines.removeAll(keepingCapacity: true)
            sawData = false
            eventType = ""
        }
        // Per spec: if the data buffer is empty (no `data:` field seen at
        // all), reset the type buffer and dispatch nothing. A lone `data:`
        // with an empty value still dispatches an empty-string event.
        guard sawData else { return nil }
        return ServerSentEvent(
            event: eventType.isEmpty ? "message" : eventType,
            data: dataLines.joined(separator: "\n"),
            lastEventID: lastEventID.isEmpty ? nil : lastEventID
        )
    }
}
