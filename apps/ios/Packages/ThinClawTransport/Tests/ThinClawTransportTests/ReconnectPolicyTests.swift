import Foundation
import Testing

@testable import ThinClawTransport

@Suite("ReconnectPolicy")
struct ReconnectPolicyTests {
    let policy = ReconnectPolicy.default

    @Test("ceiling grows exponentially from 1s")
    func ceilingGrowth() {
        #expect(policy.ceilingDelay(forAttempt: 0) == .seconds(1))
        #expect(policy.ceilingDelay(forAttempt: 1) == .seconds(2))
        #expect(policy.ceilingDelay(forAttempt: 2) == .seconds(4))
        #expect(policy.ceilingDelay(forAttempt: 5) == .seconds(32))
    }

    @Test("ceiling caps at 60s and stays there")
    func ceilingCap() {
        #expect(policy.ceilingDelay(forAttempt: 6) == .seconds(60))
        #expect(policy.ceilingDelay(forAttempt: 20) == .seconds(60))
        // Large enough to overflow pow() to +inf — must still cap cleanly.
        #expect(policy.ceilingDelay(forAttempt: 10_000) == .seconds(60))
    }

    @Test("full jitter stays within [0, ceiling] for every attempt")
    func jitterBounds() {
        var rng = SplitMix64(seed: 0xDEAD_BEEF)
        for attempt in 0..<24 {
            let ceiling = policy.ceilingDelay(forAttempt: attempt)
            for _ in 0..<64 {
                let delay = policy.delay(forAttempt: attempt, using: &rng)
                #expect(delay >= .zero)
                #expect(delay <= ceiling)
            }
        }
    }

    @Test("jitter is deterministic for a fixed seed")
    func jitterDeterminism() {
        var a = SplitMix64(seed: 42)
        var b = SplitMix64(seed: 42)
        for attempt in 0..<10 {
            #expect(
                policy.delay(forAttempt: attempt, using: &a)
                    == policy.delay(forAttempt: attempt, using: &b))
        }
    }

    @Test("jitter actually spreads (not pinned to the ceiling)")
    func jitterSpreads() {
        var rng = SplitMix64(seed: 7)
        let samples = (0..<256).map { _ in
            policy.delay(forAttempt: 6, using: &rng).timeIntervalValue
        }
        let low = samples.filter { $0 < 30 }.count
        let high = samples.filter { $0 >= 30 }.count
        // Uniform over [0, 60]: both halves must be populated.
        #expect(low > 32)
        #expect(high > 32)
    }

    @Test("system-RNG convenience overload respects bounds")
    func systemRNGOverload() {
        for attempt in [0, 3, 9] {
            let delay = policy.delay(forAttempt: attempt)
            #expect(delay >= .zero)
            #expect(delay <= policy.ceilingDelay(forAttempt: attempt))
        }
    }

    @Test("heartbeat watchdog default is 90s")
    func heartbeatWatchdogConstant() {
        #expect(policy.heartbeatTimeout == .seconds(90))
    }

    @Test("zero base delay never produces a negative or crashing delay")
    func zeroBaseDelay() {
        let degenerate = ReconnectPolicy(baseDelay: .zero, maxDelay: .zero)
        var rng = SplitMix64(seed: 1)
        #expect(degenerate.delay(forAttempt: 0, using: &rng) == .zero)
    }
}
