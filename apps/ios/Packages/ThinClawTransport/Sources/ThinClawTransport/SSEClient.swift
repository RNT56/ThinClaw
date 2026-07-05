import Foundation

/// Adapts a raw byte stream (e.g. `URLSession.AsyncBytes`) into an async
/// stream of parsed ``ServerSentEvent`` values.
///
/// The client is deliberately decoupled from networking: it consumes any
/// `AsyncSequence` of `UInt8`, so tests drive it with scripted chunked input
/// and production wraps `URLSession.bytes(for:)`. One `SSEClient` instance
/// corresponds to one logical stream subscription; it retains stream-level
/// state (`lastEventID`, server-requested retry delay) across the life of
/// the actor so a reconnect layer can consult it between attempts.
public actor SSEClient {
    private var parser = SSEParser()
    private var pumpTask: Task<Void, Never>?

    public init() {}

    /// The most recent `id:` value seen, for `Last-Event-ID` on reconnect.
    public var lastEventID: String? {
        parser.lastEventID.isEmpty ? nil : parser.lastEventID
    }

    /// Server-requested reconnection delay from the last `retry:` field.
    public var reconnectionTime: Duration? {
        parser.reconnectionTime
    }

    /// Parse `bytes` into events until the byte stream ends or throws.
    ///
    /// - The returned stream finishes when the input finishes (per the SSE
    ///   spec, a trailing half-received event is discarded, not emitted).
    /// - An error from the byte stream is rethrown to the consumer after all
    ///   events completed before the failure have been yielded.
    /// - Cancelling the consumer cancels the underlying iteration.
    /// - `onActivity` fires whenever *any* line completes — including comment
    ///   (`:`) keep-alives that produce no ``ServerSentEvent`` — so a liveness
    ///   watchdog can distinguish an idle-but-alive connection (the gateway
    ///   heartbeats with SSE comments) from true silence. It may fire without a
    ///   corresponding yielded event.
    public func events<Bytes>(
        from bytes: Bytes,
        onActivity: (@Sendable () -> Void)? = nil
    ) -> AsyncThrowingStream<ServerSentEvent, any Error>
    where Bytes: AsyncSequence & Sendable, Bytes.Element == UInt8 {
        let (stream, continuation) = AsyncThrowingStream<ServerSentEvent, any Error>
            .makeStream()

        // `Task {}` inside an actor-isolated method inherits the actor's
        // isolation, so mutating `parser` below is safe under Swift 6
        // strict concurrency.
        let task = Task {
            do {
                var lastLinesConsumed = parser.linesConsumed
                for try await byte in bytes {
                    if Task.isCancelled { break }
                    // CollectionOfOne avoids an Array allocation per byte;
                    // the parser buffers internally.
                    for event in parser.feed(CollectionOfOne(byte)) {
                        continuation.yield(event)
                    }
                    // Any completed line (comment keep-alive included) counts
                    // as stream activity, even when it produced no event.
                    if parser.linesConsumed != lastLinesConsumed {
                        lastLinesConsumed = parser.linesConsumed
                        onActivity?()
                    }
                }
                parser.finish()
                continuation.finish()
            } catch {
                parser.finish()
                continuation.finish(throwing: error)
            }
        }
        pumpTask = task
        continuation.onTermination = { _ in
            task.cancel()
        }
        return stream
    }

    /// Cancel any in-flight pump (also triggered by consumer termination).
    public func cancel() {
        pumpTask?.cancel()
        pumpTask = nil
    }
}
