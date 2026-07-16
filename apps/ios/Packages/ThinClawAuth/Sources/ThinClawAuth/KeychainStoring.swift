import Foundation

/// Errors from a keychain-backed secret store.
public enum KeychainStoreError: Error, Equatable {
    /// Underlying SecItem call failed with the given OSStatus.
    case unhandled(status: Int32)
    /// Stored payload could not be interpreted.
    case invalidData
}

public enum KeychainAccessibility: Sendable {
    case afterFirstUnlockDeviceOnly
    case whenUnlockedDeviceOnly
}

/// Minimal secret storage seam.
///
/// The real implementation is ``SecItemKeychainStore``; tests and previews
/// use ``InMemoryKeychain``. Deliberately synchronous — keychain reads are
/// fast, and callers already run credential loads off the main actor.
public protocol KeychainStoring: Sendable {
    /// Insert or replace the secret stored under `key`.
    func setSecret(_ data: Data, for key: String) throws
    func setSecret(_ data: Data, for key: String, accessibility: KeychainAccessibility) throws
    /// The secret stored under `key`, or `nil` when absent.
    func secret(for key: String) throws -> Data?
    /// Remove any secret stored under `key` (no error when absent).
    func removeSecret(for key: String) throws
}

extension KeychainStoring {
    public func setSecret(
        _ data: Data,
        for key: String,
        accessibility: KeychainAccessibility
    ) throws {
        try setSecret(data, for: key)
    }

    /// Convenience: store a Codable value as JSON.
    public func setCodable<T: Encodable>(_ value: T, for key: String) throws {
        try setSecret(JSONEncoder().encode(value), for: key)
    }

    /// Convenience: read a Codable value stored as JSON.
    public func codable<T: Decodable>(_ type: T.Type, for key: String) throws -> T? {
        guard let data = try secret(for: key) else { return nil }
        do {
            return try JSONDecoder().decode(T.self, from: data)
        } catch {
            throw KeychainStoreError.invalidData
        }
    }
}

/// Thread-safe in-memory ``KeychainStoring`` for tests and SwiftUI previews.
public final class InMemoryKeychain: KeychainStoring, @unchecked Sendable {
    // NSLock-guarded storage; @unchecked Sendable is sound because every
    // access to `storage` happens inside the lock.
    private let lock = NSLock()
    private var storage: [String: Data] = [:]

    public init() {}

    public func setSecret(_ data: Data, for key: String) throws {
        lock.withLock { storage[key] = data }
    }

    public func setSecret(
        _ data: Data,
        for key: String,
        accessibility: KeychainAccessibility
    ) throws {
        lock.withLock { storage[key] = data }
    }

    public func secret(for key: String) throws -> Data? {
        lock.withLock { storage[key] }
    }

    public func removeSecret(for key: String) throws {
        lock.withLock { storage[key] = nil }
    }

    /// Test hook: number of stored secrets.
    public var count: Int {
        lock.withLock { storage.count }
    }
}
