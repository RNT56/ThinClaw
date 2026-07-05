import Foundation
import Testing

@testable import ThinClawCore

@Suite("ComposerCooldown")
struct ComposerCooldownTests {
    let t0 = Date(timeIntervalSince1970: 1000)

    @Test("a fresh cooldown is not cooling down")
    func idle() {
        let cooldown = ComposerCooldown()
        #expect(!cooldown.isCoolingDown(now: t0))
        #expect(cooldown.remaining(now: t0) == 0)
    }

    @Test("429 with Retry-After blocks for exactly that window")
    func honorsRetryAfter() {
        var cooldown = ComposerCooldown()
        cooldown.begin(retryAfter: 30, now: t0)
        #expect(cooldown.isCoolingDown(now: t0))
        #expect(cooldown.remaining(now: t0) == 30)
        #expect(cooldown.isCoolingDown(now: t0.addingTimeInterval(29)))
        #expect(!cooldown.isCoolingDown(now: t0.addingTimeInterval(30)))
    }

    @Test("429 without Retry-After uses the default interval")
    func fallbackInterval() {
        var cooldown = ComposerCooldown()
        cooldown.begin(retryAfter: nil, now: t0)
        #expect(cooldown.remaining(now: t0) == ComposerCooldown.defaultInterval)
    }

    @Test("a second, shorter 429 does not shorten an active window")
    func doesNotShorten() {
        var cooldown = ComposerCooldown()
        cooldown.begin(retryAfter: 60, now: t0)
        cooldown.begin(retryAfter: 5, now: t0)
        #expect(cooldown.remaining(now: t0) == 60)
    }

    @Test("a later, longer 429 extends the window")
    func extends() {
        var cooldown = ComposerCooldown()
        cooldown.begin(retryAfter: 10, now: t0)
        cooldown.begin(retryAfter: 100, now: t0.addingTimeInterval(5))
        #expect(cooldown.remaining(now: t0.addingTimeInterval(5)) == 100)
    }

    @Test("clear re-enables the composer immediately")
    func clears() {
        var cooldown = ComposerCooldown()
        cooldown.begin(retryAfter: 30, now: t0)
        cooldown.clear()
        #expect(!cooldown.isCoolingDown(now: t0))
    }

    @Test("negative Retry-After is clamped to zero (never blocks)")
    func clampsNegative() {
        var cooldown = ComposerCooldown()
        cooldown.begin(retryAfter: -10, now: t0)
        #expect(!cooldown.isCoolingDown(now: t0))
    }
}
