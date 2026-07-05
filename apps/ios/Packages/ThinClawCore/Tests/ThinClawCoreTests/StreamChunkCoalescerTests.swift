import Testing

@testable import ThinClawCore

@Suite("StreamChunkCoalescer")
struct StreamChunkCoalescerTests {
    let thread = ThreadID("th_01")

    @Test("a burst of chunks folds into a single drained update")
    func chunkBurstFolds() {
        var coalescer = StreamChunkCoalescer()

        for piece in ["Hel", "lo", ", ", "world"] {
            let immediate = coalescer.reduce(.streamChunk(content: piece, threadID: thread))
            #expect(immediate == nil, "chunks must not emit per-token updates")
        }

        let update = coalescer.drain()
        #expect(update == .init(text: "Hello, world", threadID: thread, isFinal: false))
    }

    @Test("drain is idempotent until new chunks arrive")
    func drainClearsDirtyFlag() {
        var coalescer = StreamChunkCoalescer()
        coalescer.reduce(.streamChunk(content: "a", threadID: thread))

        #expect(coalescer.drain() != nil)
        #expect(coalescer.drain() == nil)

        coalescer.reduce(.streamChunk(content: "b", threadID: thread))
        #expect(coalescer.drain()?.text == "ab")
    }

    @Test("response finalizes immediately, preferring the full response body")
    func responseFinalizes() {
        var coalescer = StreamChunkCoalescer()
        coalescer.reduce(.streamChunk(content: "Hel", threadID: thread))
        coalescer.reduce(.streamChunk(content: "lo", threadID: thread))

        let final = coalescer.reduce(.response(content: "Hello.", threadID: thread))
        #expect(final == .init(text: "Hello.", threadID: thread, isFinal: true))

        // Fully reset afterwards.
        #expect(coalescer.drain() == nil)
        #expect(coalescer.pendingText.isEmpty)
    }

    @Test("empty response body falls back to accumulated chunks")
    func emptyResponseFallsBack() {
        var coalescer = StreamChunkCoalescer()
        coalescer.reduce(.streamChunk(content: "partial answer", threadID: thread))

        let final = coalescer.reduce(.response(content: "", threadID: thread))
        #expect(final?.text == "partial answer")
        #expect(final?.isFinal == true)
    }

    @Test("error flushes partial streamed text instead of dropping it")
    func errorFlushesPartialText() {
        var coalescer = StreamChunkCoalescer()
        coalescer.reduce(.streamChunk(content: "half a thou", threadID: thread))

        let flushed = coalescer.reduce(.error(message: "LLM provider timeout", threadID: thread))
        #expect(flushed == .init(text: "half a thou", threadID: thread, isFinal: true))
    }

    @Test("error with nothing streamed emits nothing")
    func errorWithoutBufferIsSilent() {
        var coalescer = StreamChunkCoalescer()
        #expect(coalescer.reduce(.error(message: "boom", threadID: thread)) == nil)
    }

    @Test("interleaved non-chunk events do not disturb accumulation")
    func passThroughEventsDoNotDisturb() {
        var coalescer = StreamChunkCoalescer()
        coalescer.reduce(.streamChunk(content: "abc", threadID: thread))
        coalescer.reduce(.heartbeat)
        coalescer.reduce(.thinking(message: "Running tool", threadID: thread))
        coalescer.reduce(.toolStarted(name: "shell", threadID: thread))
        coalescer.reduce(
            .usageUpdate(UsageUpdate(inputTokens: 10, outputTokens: 3, threadID: thread)))
        coalescer.reduce(.unknown(type: "plan_update"))
        coalescer.reduce(.streamChunk(content: "def", threadID: thread))

        #expect(coalescer.drain()?.text == "abcdef")
    }

    @Test("thread id is adopted from the first chunk that carries one")
    func threadIDAdoption() {
        var coalescer = StreamChunkCoalescer()
        coalescer.reduce(.streamChunk(content: "a", threadID: nil))
        coalescer.reduce(.streamChunk(content: "b", threadID: thread))

        #expect(coalescer.drain()?.threadID == thread)
    }

    @Test("response with nil thread id keeps the accumulated thread id")
    func finalKeepsAccumulatedThreadID() {
        var coalescer = StreamChunkCoalescer()
        coalescer.reduce(.streamChunk(content: "x", threadID: thread))

        let final = coalescer.reduce(.response(content: "x!", threadID: nil))
        #expect(final?.threadID == thread)
    }
}
