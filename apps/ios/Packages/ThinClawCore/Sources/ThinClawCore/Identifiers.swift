import Foundation

/// Strongly typed identifier for a chat thread (conversation) on the gateway.
///
/// The gateway uses opaque string thread ids (e.g. `"web-1720000000"`); this
/// newtype exists so thread ids, message ids, and free-form strings cannot be
/// confused at compile time.
public struct ThreadID: Hashable, Sendable, Codable, CustomStringConvertible {
    public let rawValue: String

    public init(_ rawValue: String) {
        self.rawValue = rawValue
    }

    public init(from decoder: any Decoder) throws {
        self.rawValue = try decoder.singleValueContainer().decode(String.self)
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }

    public var description: String { rawValue }
}

/// Strongly typed identifier for a single message / timeline item.
///
/// Client-generated (UUID-based) for locally created items; server ids are
/// adopted verbatim when the gateway supplies them.
public struct MessageID: Hashable, Sendable, Codable, CustomStringConvertible {
    public let rawValue: String

    public init(_ rawValue: String) {
        self.rawValue = rawValue
    }

    /// A fresh, unique, client-generated id.
    public init() {
        self.rawValue = UUID().uuidString
    }

    public init(from decoder: any Decoder) throws {
        self.rawValue = try decoder.singleValueContainer().decode(String.self)
    }

    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }

    public var description: String { rawValue }
}
