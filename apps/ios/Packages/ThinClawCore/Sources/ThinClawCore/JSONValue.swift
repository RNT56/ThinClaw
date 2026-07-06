import Foundation

/// A minimal, `Decodable` representation of an arbitrary JSON value.
///
/// Job events carry an opaque `data` object (`serde_json::Value` on the
/// gateway) whose shape varies by event type. Rather than pull in a heavy JSON
/// dependency or decode every payload variant, the tail decodes `data` into
/// this value and reads named fields defensively via ``string(for:)`` /
/// ``bool(for:)``. Kept in ThinClawCore so the projection stays macOS-testable.
public enum JSONValue: Decodable, Hashable, Sendable {
    case object([String: JSONValue])
    case array([JSONValue])
    case string(String)
    case number(Double)
    case bool(Bool)
    case null

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([String: JSONValue].self) {
            self = .object(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            throw DecodingError.dataCorruptedError(
                in: container, debugDescription: "Unrecognized JSON value")
        }
    }

    /// The string at `key` when this is an object whose `key` holds a string.
    public func string(for key: String) -> String? {
        guard case let .object(fields) = self, case let .string(value)? = fields[key]
        else { return nil }
        return value
    }

    /// The bool at `key` when this is an object whose `key` holds a bool.
    public func bool(for key: String) -> Bool? {
        guard case let .object(fields) = self, case let .bool(value)? = fields[key]
        else { return nil }
        return value
    }
}
