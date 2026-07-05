import Foundation
import Testing

@testable import ThinClawTransport

// MARK: - Deterministic RNG

/// SplitMix64 — tiny, deterministic RNG for reproducible jitter tests.
struct SplitMix64: RandomNumberGenerator {
    private var state: UInt64

    init(seed: UInt64) {
        self.state = seed
    }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}

// MARK: - Scripted byte stream

/// An `AsyncSequence<UInt8>` that replays pre-scripted chunks, optionally
/// throwing after the last chunk — the networking stand-in for SSEClient
/// tests.
struct ScriptedByteStream: AsyncSequence, Sendable {
    typealias Element = UInt8

    let chunks: [[UInt8]]
    let finalError: (any Error)?

    init(chunks: [[UInt8]], finalError: (any Error)? = nil) {
        self.chunks = chunks
        self.finalError = finalError
    }

    init(text: String, chunkSize: Int = .max, finalError: (any Error)? = nil) {
        self.chunks = Array(text.utf8).chunked(into: chunkSize)
        self.finalError = finalError
    }

    struct AsyncIterator: AsyncIteratorProtocol {
        var flattened: [UInt8]
        var index = 0
        let finalError: (any Error)?

        mutating func next() async throws -> UInt8? {
            if index < flattened.count {
                defer { index += 1 }
                return flattened[index]
            }
            if let finalError { throw finalError }
            return nil
        }
    }

    func makeAsyncIterator() -> AsyncIterator {
        AsyncIterator(flattened: chunks.flatMap { $0 }, finalError: finalError)
    }
}

struct ScriptedStreamError: Error, Equatable {}

// MARK: - Chunking helpers

extension Array where Element == UInt8 {
    /// Split into fixed-size chunks (last chunk may be short).
    func chunked(into size: Int) -> [[UInt8]] {
        guard size < count else { return isEmpty ? [] : [self] }
        precondition(size > 0)
        return stride(from: 0, to: count, by: size).map {
            Array(self[$0..<Swift.min($0 + size, count)])
        }
    }
}

/// Feed `bytes` to a fresh parser in chunks of `size`, collecting all events.
func parseChunked(_ bytes: [UInt8], size: Int) -> [ServerSentEvent] {
    var parser = SSEParser()
    var events: [ServerSentEvent] = []
    for chunk in bytes.chunked(into: size) {
        events.append(contentsOf: parser.feed(chunk))
    }
    parser.finish()
    return events
}

/// The chunk sizes every fixture is replayed at: byte-by-byte, several prime
/// widths that guarantee mid-line and mid-UTF-8 splits, and all-at-once.
let adversarialChunkSizes: [Int] = [1, 2, 3, 5, 7, 16, 64, .max]

// MARK: - Fixtures

enum Fixture: String, CaseIterable {
    case basic = "chat-stream-basic"
    case toolsApproval = "chat-stream-tools-approval"
    case unknownAndComments = "chat-stream-unknown-and-comments"
    case utf8 = "chat-stream-utf8"

    func bytes() throws -> [UInt8] {
        let url = try #require(
            Bundle.module.url(
                forResource: rawValue, withExtension: "sse", subdirectory: "Fixtures"),
            "missing fixture \(rawValue).sse")
        return Array(try Data(contentsOf: url))
    }

    /// The fixture with LF line endings rewritten to CRLF.
    func crlfBytes() throws -> [UInt8] {
        var out: [UInt8] = []
        for byte in try bytes() {
            if byte == 0x0A { out.append(0x0D) }
            out.append(byte)
        }
        return out
    }
}
