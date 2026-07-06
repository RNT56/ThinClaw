import Foundation

/// A device scope, as granted by the gateway. Mirrors the gateway's
/// `DeviceScope` (`chat`, `approvals`, `jobs:read`, `devices:self`) but is a
/// plain Foundation enum so the settings surface is testable on macOS without
/// linking the generated `ThinClawAPI` client. Unknown wire strings are
/// preserved verbatim in ``other`` so a newer gateway grant still renders.
public enum DeviceScopeTag: Hashable, Sendable {
    case chat
    case approvals
    case jobsRead
    case devicesSelf
    case other(String)

    /// Parse a gateway wire string (`"chat"`, `"jobs:read"`, …).
    public init(wire: String) {
        switch wire {
        case "chat": self = .chat
        case "approvals": self = .approvals
        case "jobs:read": self = .jobsRead
        case "devices:self": self = .devicesSelf
        default: self = .other(wire)
        }
    }

    /// Short human label for a scope chip.
    public var label: String {
        switch self {
        case .chat: return "Chat"
        case .approvals: return "Approvals"
        case .jobsRead: return "Jobs"
        case .devicesSelf: return "Device management"
        case .other(let raw): return raw
        }
    }
}

/// A device as surfaced to the settings screen — the public projection the
/// gateway returns from `GET /api/devices/me` and `GET /api/devices/me/companions`
/// (`DeviceInfo`), minus every token/hash field. UI-free and macOS-testable; the
/// `FeatureSettings` adapter maps the generated `DeviceInfo` into this.
public struct ManagedDevice: Identifiable, Hashable, Sendable {
    /// Server-assigned device id (`device_id`).
    public var id: String
    /// Operator-facing device name.
    public var name: String
    /// Platform label (`ios`, `ipados`, `watchos`, `macos`, or a raw string).
    public var platform: String
    /// Granted scopes, in gateway order.
    public var scopes: [DeviceScopeTag]
    /// `last_seen_at` as an RFC3339 string (kept as text — the gateway is the
    /// clock authority; the UI formats it relative to now for display).
    public var lastSeenAt: String
    /// `token_prefix`: the non-secret leading bytes of the token (e.g. `tcd_ab`),
    /// shown so the operator can correlate a row with a token without ever
    /// surfacing the secret.
    public var tokenPrefix: String
    /// Parent device id when this device is a companion (a watch); `nil` for a
    /// top-level paired device.
    public var parentDeviceID: String?

    public init(
        id: String,
        name: String,
        platform: String,
        scopes: [DeviceScopeTag],
        lastSeenAt: String,
        tokenPrefix: String,
        parentDeviceID: String? = nil
    ) {
        self.id = id
        self.name = name
        self.platform = platform
        self.scopes = scopes
        self.lastSeenAt = lastSeenAt
        self.tokenPrefix = tokenPrefix
        self.parentDeviceID = parentDeviceID
    }

    /// Whether this device is a companion (has a parent) — i.e. the paired
    /// watch, which the operator revokes from the phone.
    public var isCompanion: Bool { parentDeviceID != nil }
}

/// The device-management network operations the settings surface needs,
/// abstracted so ``SettingsStore`` is exercisable on macOS with a mocked client.
///
/// Every call is scoped to the **current** device's token (`devices:self`): the
/// gateway resolves "me" from the bearer, so there is no device id to pass for
/// self-inspection. The production adapter (`GatewayDeviceManager` in
/// `FeatureSettings`) wraps the generated `ThinClawAPI` client over the pinned
/// session.
///
/// Note on omitted operations: the gateway exposes **no** device self-rename or
/// self-rotate route — `POST /api/devices/{id}/rename` and
/// `POST /api/devices/{id}/rotate` are admin-only (they reject a device token
/// via the `devices:self` scope mapping). The phone therefore can rename/rotate
/// nothing about itself over its own token, so those actions are deliberately
/// absent from this seam rather than being wired to the admin path.
public protocol DeviceManaging: Sendable {
    /// This device's own record (`GET /api/devices/me`).
    func thisDevice() async throws -> ManagedDevice

    /// The current device's companions (`GET /api/devices/me/companions`) — the
    /// paired watch(es). Empty when none are provisioned.
    func companions() async throws -> [ManagedDevice]

    /// Revoke a companion by id (`DELETE /api/devices/me/companions/{id}`). This
    /// is how the operator de-authorizes their watch from the phone; the gateway
    /// cascade-clears the companion's push/Live-Activity registrations and tears
    /// down its live streams.
    func revokeCompanion(id: String) async throws
}
