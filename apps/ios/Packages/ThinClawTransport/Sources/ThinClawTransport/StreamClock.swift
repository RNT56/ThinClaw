import Foundation

/// The time seam for ``GatewayStream``: backoff sleeps and the heartbeat
/// watchdog measure duration through this, so tests can drive the reconnect
/// state machine deterministically without wall-clock waits.
///
/// Production uses ``SystemStreamClock`` (a `ContinuousClock` wrapper). Tests
/// supply a manual clock whose `sleep` resolves immediately (or on demand) and
/// whose `now` they advance by hand.
public protocol StreamClock: Sendable {
    /// Suspend for `duration`, honoring cancellation.
    func sleep(for duration: Duration) async throws
    /// A monotonic instant in seconds since an arbitrary epoch, for measuring
    /// heartbeat gaps. Only differences are meaningful.
    func nowSeconds() -> Double
}

/// Wall-clock ``StreamClock`` built on `ContinuousClock`.
public struct SystemStreamClock: StreamClock {
    private let clock = ContinuousClock()
    /// Fixed reference instant; `nowSeconds` reports elapsed time from it so
    /// successive calls are comparable (`ContinuousClock.Instant` has no epoch
    /// conversion of its own).
    private let epoch: ContinuousClock.Instant

    public init() {
        self.epoch = clock.now
    }

    public func sleep(for duration: Duration) async throws {
        try await Task.sleep(for: duration, clock: clock)
    }

    public func nowSeconds() -> Double {
        epoch.duration(to: clock.now).timeIntervalValue
    }
}
