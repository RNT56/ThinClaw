import Foundation

/// Pure reconnect policy for the gateway event stream: exponential backoff
/// with full jitter, plus the heartbeat-watchdog constants.
///
/// "Full jitter" (AWS-style): the actual delay for attempt *n* is drawn
/// uniformly from `[0, min(maxDelay, baseDelay * multiplier^n)]`. This
/// spreads thundering herds after a gateway restart across the whole window
/// instead of synchronizing retries at the ceiling.
///
/// This is a value type with no clocks or timers — callers supply the RNG
/// (tests use a seeded generator) and perform the actual sleeping.
public struct ReconnectPolicy: Hashable, Sendable {
    /// Backoff ceiling for attempt 0.
    public var baseDelay: Duration
    /// Absolute ceiling for any attempt.
    public var maxDelay: Duration
    /// Ceiling growth factor per attempt.
    public var multiplier: Double
    /// If no event (including `heartbeat`) arrives for this long on an open
    /// stream, the watchdog should treat the connection as dead and
    /// reconnect. The gateway heartbeats well inside this window.
    public var heartbeatTimeout: Duration

    /// ThinClaw defaults: 1s -> 60s backoff, 90s heartbeat watchdog.
    public static let `default` = ReconnectPolicy(
        baseDelay: .seconds(1),
        maxDelay: .seconds(60),
        multiplier: 2,
        heartbeatTimeout: .seconds(90)
    )

    public init(
        baseDelay: Duration,
        maxDelay: Duration,
        multiplier: Double = 2,
        heartbeatTimeout: Duration = .seconds(90)
    ) {
        self.baseDelay = baseDelay
        self.maxDelay = maxDelay
        self.multiplier = multiplier
        self.heartbeatTimeout = heartbeatTimeout
    }

    /// The jitter window ceiling for a given 0-based attempt number:
    /// `min(maxDelay, baseDelay * multiplier^attempt)`.
    public func ceilingDelay(forAttempt attempt: Int) -> Duration {
        precondition(attempt >= 0, "attempt is 0-based and non-negative")
        let base = baseDelay.timeIntervalValue
        let cap = maxDelay.timeIntervalValue
        let raw = base * pow(multiplier, Double(attempt))
        // `raw` can overflow to +inf for large attempts; min() handles it.
        return .seconds(min(cap, raw))
    }

    /// Full-jitter delay for a given attempt, drawn from the supplied RNG.
    public func delay(
        forAttempt attempt: Int,
        using generator: inout some RandomNumberGenerator
    ) -> Duration {
        let ceiling = ceilingDelay(forAttempt: attempt).timeIntervalValue
        guard ceiling > 0 else { return .zero }
        return .seconds(Double.random(in: 0...ceiling, using: &generator))
    }

    /// Full-jitter delay using the system RNG.
    public func delay(forAttempt attempt: Int) -> Duration {
        var generator = SystemRandomNumberGenerator()
        return delay(forAttempt: attempt, using: &generator)
    }
}

extension Duration {
    /// Seconds as a `Double` (attosecond-lossy, fine for backoff math).
    var timeIntervalValue: TimeInterval {
        let (seconds, attoseconds) = components
        return Double(seconds) + Double(attoseconds) / 1e18
    }
}
