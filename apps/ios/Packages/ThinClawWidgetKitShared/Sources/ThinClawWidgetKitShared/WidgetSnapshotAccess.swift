import Foundation
import ThinClawSnapshotKit

/// Well-known App Group / snapshot wiring shared by the widget timeline
/// providers and the interactive intents.
///
/// The App Group `group.com.thinclaw.shared` holds non-secret widget state
/// (gateway URL, instance id, pin, and the snapshot files) per
/// docs/MOBILE_SECURITY.md **D-K2**. Secrets (the `tcd_…` device token) live
/// in the shared Keychain access group, never here — see
/// ``ThinClawAuth.SharedGatewayConnection``.
public enum WidgetSnapshotAccess {
    /// The App Group container id shared by the app, widgets, and NSE
    /// (docs/MOBILE_SECURITY.md D-K2). Snapshot files live under
    /// `<container>/Snapshots/`.
    public static let appGroupID = "group.com.thinclaw.shared"

    /// The shared snapshot store, or `nil` when the App Group container is
    /// unavailable (missing entitlement, or a plain test host). Timeline
    /// providers treat `nil` as "not connected" and render a placeholder —
    /// they never crash.
    public static func store() -> SnapshotStore? {
        SnapshotStore(appGroupID: appGroupID)
    }

    /// Load a snapshot from the shared store, collapsing every failure —
    /// missing container, missing file, corrupt payload, newer schema — into
    /// `nil` so a timeline provider always renders and never throws.
    ///
    /// Schema/corruption failures are intentionally swallowed here: a widget
    /// that outlives an app upgrade should degrade to a "stale / open app"
    /// placeholder, not crash the extension process.
    public static func load<S: SharedSnapshot>(_ type: S.Type) -> S? {
        guard let store = store() else { return nil }
        return (try? store.load(type)) ?? nil
    }
}

extension SharedSnapshot {
    /// Whether this snapshot is older than `maxAge` (default 30 min), so a
    /// glance surface can honestly badge it "stale as of <generatedAt>"
    /// instead of implying live data.
    public func isStale(asOf now: Date = .now, maxAge: TimeInterval = 30 * 60) -> Bool {
        now.timeIntervalSince(generatedAt) > maxAge
    }
}
