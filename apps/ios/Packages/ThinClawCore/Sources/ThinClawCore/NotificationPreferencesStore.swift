import Foundation

/// A minimal string→string key/value store, abstracted so the preferences round
/// trip is testable on macOS without an App Group container. The production
/// adapter wraps `UserDefaults(suiteName:)` on the shared App Group suite so the
/// Notification Service Extension reads the same values the app writes.
public protocol KeyValueStoring: Sendable {
    func string(forKey key: String) -> String?
    func set(_ value: String?, forKey key: String)
}

/// An in-memory ``KeyValueStoring`` for tests (and as a safe fallback when the
/// App Group container is unavailable). Thread-safe via an internal lock so it
/// satisfies `Sendable` like the real defaults-backed store.
public final class InMemoryKeyValueStore: KeyValueStoring, @unchecked Sendable {
    private let lock = NSLock()
    private var storage: [String: String]

    public init(_ initial: [String: String] = [:]) {
        self.storage = initial
    }

    public func string(forKey key: String) -> String? {
        lock.lock()
        defer { lock.unlock() }
        return storage[key]
    }

    public func set(_ value: String?, forKey key: String) {
        lock.lock()
        defer { lock.unlock() }
        if let value {
            storage[key] = value
        } else {
            storage.removeValue(forKey: key)
        }
    }
}

/// Reads and writes the operator's per-category notification preview preferences
/// (D-N3) through a ``KeyValueStoring`` backed by the shared App Group defaults,
/// so the Notification Service Extension can consult the exact values the
/// settings UI persisted.
///
/// The keys are stable, App-Group-scoped strings (namespaced under
/// `notif.preview.<category>`) so the NSE — which links no app code — can read a
/// single key with `UserDefaults(suiteName:).string(forKey:)` without importing
/// this type. An unset or unrecognized value falls back to the category default.
public struct NotificationPreferencesStore: Sendable {
    private let store: any KeyValueStoring

    public init(store: any KeyValueStoring) {
        self.store = store
    }

    /// The App Group defaults key for a category's preview mode. Stable wire
    /// contract shared with the NSE.
    public static func key(for category: NotificationCategory) -> String {
        "notif.preview.\(category.rawValue)"
    }

    /// Load the persisted preferences, defaulting any unset/invalid category.
    public func load() -> NotificationPreferences {
        NotificationPreferences(
            message: mode(for: .message),
            approval: mode(for: .approval),
            job: mode(for: .job))
    }

    /// Persist every category from `preferences`.
    public func save(_ preferences: NotificationPreferences) {
        for category in NotificationCategory.allCases {
            store.set(
                preferences.mode(for: category).rawValue,
                forKey: Self.key(for: category))
        }
    }

    /// The persisted (or default) preview mode for a single category. This is the
    /// exact read the NSE performs per push.
    public func mode(for category: NotificationCategory) -> PreviewMode {
        guard
            let raw = store.string(forKey: Self.key(for: category)),
            let parsed = PreviewMode(rawValue: raw),
            NotificationPreferences.allowedModes(for: category).contains(parsed)
        else {
            return NotificationPreferences.default.mode(for: category)
        }
        return parsed
    }
}
