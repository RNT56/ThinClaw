import Foundation
import ThinClawAPI
import ThinClawCore

@testable import ThinClawTransport

// MARK: - Test clock

/// A ``StreamClock`` built on the same `ContinuousClock` as production but
/// exposed to tests so scenarios can run against real — but tiny — durations
/// (millisecond backoff, sub-second watchdog windows). This keeps the
/// reconnect/watchdog state machine exercised end-to-end without racy manual
/// time juggling, while staying fast and deterministic in ordering because the
/// scripted provider controls exactly when bytes arrive, end, or hang.
typealias TestClock = SystemStreamClock

// MARK: - Scripted byte-stream provider

/// A ``ByteStreamProvider`` that hands out pre-scripted connections in order.
///
/// Each call to ``open(token:)`` consumes the next scripted connection: it can
/// throw (simulating a failed connect), or yield a byte stream that emits the
/// scripted SSE text and then either ends (server EOF), stays silent forever
/// (to let the watchdog trip), or throws mid-stream (transport drop).
final class ScriptedProvider: ByteStreamProvider, @unchecked Sendable {
    enum Connection: Sendable {
        /// Emit this SSE text, then end cleanly (server EOF).
        case emit(String)
        /// Emit this SSE text, then throw (transport drop).
        case emitThenError(String)
        /// Emit this SSE text, then hang forever (watchdog territory).
        case emitThenHang(String)
        /// Emit `line` every `interval`, `count` times (simulating periodic
        /// keep-alives such as the gateway's SSE comment heartbeat), then hang
        /// forever. With a comment `line` this produces no decoded events yet
        /// keeps the byte stream demonstrably alive.
        case drip(line: String, every: Duration, count: Int)
        /// Fail the connect itself.
        case failConnect
    }

    private let connections: [Connection]
    private let lock = NSLock()
    private var index = 0
    private(set) var tokensSeen: [String] = []

    init(_ connections: [Connection]) {
        self.connections = connections
    }

    var openCount: Int {
        lock.withLock { index }
    }

    func open(token: String) async throws -> ByteStream {
        let connection: Connection = try lock.withLock {
            guard index < connections.count else {
                // Exhausted: hang so the supervisor is quiescent until shutdown.
                index += 1
                tokensSeen.append(token)
                return .emitThenHang("")
            }
            let next = connections[index]
            index += 1
            tokensSeen.append(token)
            return next
        }

        switch connection {
        case .failConnect:
            throw GatewayStreamError.http(status: 500)
        case .emit(let text):
            return ByteStream(ScriptedBytes(text: text, tail: .end))
        case .emitThenError(let text):
            return ByteStream(ScriptedBytes(text: text, tail: .error))
        case .emitThenHang(let text):
            return ByteStream(ScriptedBytes(text: text, tail: .hang))
        case .drip(let line, let interval, let count):
            return ByteStream(DrippingBytes(line: line, interval: interval, count: count))
        }
    }
}

/// A byte sequence that emits `line` (a full SSE line, terminated with `\n`)
/// once every `interval`, `count` times, then hangs forever. Models a server
/// that keeps a connection alive with periodic keep-alives.
struct DrippingBytes: AsyncSequence, Sendable {
    let line: String
    let interval: Duration
    let count: Int

    struct Iterator: AsyncIteratorProtocol {
        let lineBytes: [UInt8]
        let interval: Duration
        let count: Int
        var startedLines = 0
        var byteIndex: Int

        init(lineBytes: [UInt8], interval: Duration, count: Int, byteIndex: Int) {
            self.lineBytes = lineBytes
            self.interval = interval
            self.count = count
            self.byteIndex = byteIndex
        }

        mutating func next() async throws -> UInt8? {
            // Still emitting bytes of the current line.
            if byteIndex < lineBytes.count {
                defer { byteIndex += 1 }
                return lineBytes[byteIndex]
            }
            // Between lines: if more keep-alives are scheduled, sleep one
            // interval then start the next line; otherwise hang until cancelled.
            if startedLines < count {
                try await Task.sleep(for: interval)
                startedLines += 1
                byteIndex = 1
                return lineBytes.isEmpty ? nil : lineBytes[0]
            }
            try await Task.sleep(for: .seconds(3600))
            return nil
        }
    }

    func makeAsyncIterator() -> Iterator {
        let bytes = Array(line.utf8)
        // Start "between lines" (byteIndex past the end) so the first line also
        // waits a full interval — every keep-alive is time-spaced.
        return Iterator(lineBytes: bytes, interval: interval, count: count, byteIndex: bytes.count)
    }
}

/// A byte sequence that emits `text`, then does one of: end, throw, or hang.
struct ScriptedBytes: AsyncSequence, Sendable {
    enum Tail: Sendable {
        case end
        case error
        case hang
    }

    let bytes: [UInt8]
    let tail: Tail

    init(text: String, tail: Tail) {
        self.bytes = Array(text.utf8)
        self.tail = tail
    }

    struct Iterator: AsyncIteratorProtocol {
        var bytes: [UInt8]
        var index = 0
        let tail: Tail

        mutating func next() async throws -> UInt8? {
            if index < bytes.count {
                defer { index += 1 }
                return bytes[index]
            }
            switch tail {
            case .end:
                return nil
            case .error:
                throw ScriptedTailError()
            case .hang:
                // Suspend until the surrounding task is cancelled.
                try await Task.sleep(for: .seconds(3600))
                return nil
            }
        }
    }

    func makeAsyncIterator() -> Iterator {
        Iterator(bytes: bytes, tail: tail)
    }
}

struct ScriptedTailError: Error, Equatable {}

// MARK: - Mock gateway REST client

/// A minimal ``APIProtocol`` stub. Only the chat operations the session uses
/// are implemented; the rest trap (they are never called in these tests).
final class MockGatewayClient: APIProtocol, @unchecked Sendable {
    private let lock = NSLock()
    private(set) var sentMessages: [(content: String, thread: String?)] = []
    private(set) var abortedThreads: [String?] = []
    /// Query parameters captured from each `chatHistoryHandler` call, so tests
    /// can assert the session forwards thread/before/limit (rather than
    /// silently dropping them, which was the original bug).
    private(set) var historyQueries: [Operations.ChatHistoryHandler.Input.Query] = []

    var sendResponse = Components.Schemas.SendMessageResponse(messageId: "m-1", status: "accepted")
    var threadsResponse = Components.Schemas.ThreadListResponse(threads: [])
    var historyResponse = Components.Schemas.HistoryResponse(threadId: "t1", turns: [])

    func chatSendHandler(
        _ input: Operations.ChatSendHandler.Input
    ) async throws -> Operations.ChatSendHandler.Output {
        guard case let .json(body) = input.body else { throw MockError.unexpected }
        lock.withLock { sentMessages.append((body.content, body.threadId)) }
        return .accepted(.init(body: .json(sendResponse)))
    }

    func chatAbortHandler(
        _ input: Operations.ChatAbortHandler.Input
    ) async throws -> Operations.ChatAbortHandler.Output {
        guard case let .json(body) = input.body else { throw MockError.unexpected }
        lock.withLock { abortedThreads.append(body.threadId) }
        return .accepted(.init(body: .json(.init(messageId: "", status: "aborted"))))
    }

    func chatThreadsHandler(
        _ input: Operations.ChatThreadsHandler.Input
    ) async throws -> Operations.ChatThreadsHandler.Output {
        .ok(.init(body: .json(threadsResponse)))
    }

    func chatHistoryHandler(
        _ input: Operations.ChatHistoryHandler.Input
    ) async throws -> Operations.ChatHistoryHandler.Output {
        lock.withLock { historyQueries.append(input.query) }
        return .ok(.init(body: .json(historyResponse)))
    }

    enum MockError: Error { case unexpected, unimplemented }

    // Unused operations trap loudly if a test ever reaches them.
    func chatApprovalHandler(_ input: Operations.ChatApprovalHandler.Input) async throws
        -> Operations.ChatApprovalHandler.Output
    { throw MockError.unimplemented }
    func chatApprovalsHandler(_ input: Operations.ChatApprovalsHandler.Input) async throws
        -> Operations.ChatApprovalsHandler.Output
    { throw MockError.unimplemented }
    func chatNewThreadHandler(_ input: Operations.ChatNewThreadHandler.Input) async throws
        -> Operations.ChatNewThreadHandler.Output
    { throw MockError.unimplemented }
    func chatDeleteThreadHandler(_ input: Operations.ChatDeleteThreadHandler.Input) async throws
        -> Operations.ChatDeleteThreadHandler.Output
    { throw MockError.unimplemented }
    func devicesListHandler(_ input: Operations.DevicesListHandler.Input) async throws
        -> Operations.DevicesListHandler.Output
    { throw MockError.unimplemented }
    func devicesMeHandler(_ input: Operations.DevicesMeHandler.Input) async throws
        -> Operations.DevicesMeHandler.Output
    { throw MockError.unimplemented }
    func devicesPairCompleteHandler(_ input: Operations.DevicesPairCompleteHandler.Input)
        async throws -> Operations.DevicesPairCompleteHandler.Output
    { throw MockError.unimplemented }
    func devicesPairPendingHandler(_ input: Operations.DevicesPairPendingHandler.Input)
        async throws -> Operations.DevicesPairPendingHandler.Output
    { throw MockError.unimplemented }
    func devicesPairStartHandler(_ input: Operations.DevicesPairStartHandler.Input) async throws
        -> Operations.DevicesPairStartHandler.Output
    { throw MockError.unimplemented }
    func devicesPairApproveHandler(_ input: Operations.DevicesPairApproveHandler.Input)
        async throws -> Operations.DevicesPairApproveHandler.Output
    { throw MockError.unimplemented }
    func devicesRenameHandler(_ input: Operations.DevicesRenameHandler.Input) async throws
        -> Operations.DevicesRenameHandler.Output
    { throw MockError.unimplemented }
    func devicesRevokeHandler(_ input: Operations.DevicesRevokeHandler.Input) async throws
        -> Operations.DevicesRevokeHandler.Output
    { throw MockError.unimplemented }
    func devicesRotateHandler(_ input: Operations.DevicesRotateHandler.Input) async throws
        -> Operations.DevicesRotateHandler.Output
    { throw MockError.unimplemented }
    func gatewayStatusHandler(_ input: Operations.GatewayStatusHandler.Input) async throws
        -> Operations.GatewayStatusHandler.Output
    { throw MockError.unimplemented }
    func healthHandler(_ input: Operations.HealthHandler.Input) async throws
        -> Operations.HealthHandler.Output
    { throw MockError.unimplemented }
    func jobsListHandler(_ input: Operations.JobsListHandler.Input) async throws
        -> Operations.JobsListHandler.Output
    { throw MockError.unimplemented }
    func jobsSummaryHandler(_ input: Operations.JobsSummaryHandler.Input) async throws
        -> Operations.JobsSummaryHandler.Output
    { throw MockError.unimplemented }
    func jobsDetailHandler(_ input: Operations.JobsDetailHandler.Input) async throws
        -> Operations.JobsDetailHandler.Output
    { throw MockError.unimplemented }
}
