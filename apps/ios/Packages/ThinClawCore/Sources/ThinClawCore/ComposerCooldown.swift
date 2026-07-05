import Foundation

/// Pure model of the chat composer's send cooldown after a `429 Too Many
/// Requests` from the gateway.
///
/// The gateway rate-limits chat sends and may return a `Retry-After`; the
/// composer disables its send button until that window elapses. Keeping the
/// window arithmetic in a value type here (rather than inside the iOS-only
/// `@Observable` store) means the 429 behavior is unit-tested on macOS.
///
/// A `nil` `Retry-After` falls back to ``ComposerCooldown/defaultInterval`` so
/// the composer is never left disabled forever nor re-enabled instantly.
public struct ComposerCooldown: Hashable, Sendable {
    /// Fallback cooldown when the gateway sends a 429 without a `Retry-After`.
    public static let defaultInterval: TimeInterval = 5

    /// When the current cooldown ends, or `nil` if the composer is not cooling
    /// down.
    public private(set) var until: Date?

    public init(until: Date? = nil) {
        self.until = until
    }

    /// Begin (or extend) a cooldown after a 429. `retryAfter` is the gateway's
    /// hint in seconds; `now` is the reference instant (a clock seam for tests).
    ///
    /// If a longer cooldown is already in effect it is preserved — a second 429
    /// must never shorten the window.
    public mutating func begin(retryAfter: TimeInterval?, now: Date) {
        let interval = max(0, retryAfter ?? Self.defaultInterval)
        let candidate = now.addingTimeInterval(interval)
        if let existing = until, existing > candidate { return }
        until = candidate
    }

    /// Whether sending is currently blocked at `now`. Auto-clears once the
    /// window has passed.
    public func isCoolingDown(now: Date) -> Bool {
        guard let until else { return false }
        return until > now
    }

    /// Seconds remaining at `now` (0 when not cooling down), for a countdown UI.
    public func remaining(now: Date) -> TimeInterval {
        guard let until, until > now else { return 0 }
        return until.timeIntervalSince(now)
    }

    /// Clear the cooldown (e.g. after a successful send).
    public mutating func clear() {
        until = nil
    }
}
