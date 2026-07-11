import Foundation

/// The time seam for ``GatewayStream``: backoff sleeps and the heartbeat
/// watchdog measure duration through this, keeping timing policy injectable
/// and the reconnect state machine independent of a concrete clock.
///
/// Production uses ``SystemStreamClock`` (a `ContinuousClock` wrapper). Tests
/// inject this abstraction with short policy durations so reconnect behavior
/// can be exercised without production-length waits.
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
