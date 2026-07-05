import Foundation

/// The locator hints a ThinClaw gateway advertises in its `_thinclaw._tcp` TXT
/// record (milestone B3). Parsed out of `NWBrowser`/`NWTXTRecord` bytes into a
/// value type so the (network-free) parsing can be unit-tested on its own.
///
/// The record is a **locator only** (docs/MOBILE_SECURITY.md D-X3 / T11): it
/// carries a *non-reversible fingerprint* of the gateway instance id, never the
/// raw instance id, a token, or a secret. Discovery finds candidate endpoints;
/// the app must still verify the pinned SPKI and the pairing-time instance id
/// before any credential is sent. Nothing here authenticates a gateway.
///
/// Wire keys (all optional; unknown keys ignored, forward compatible):
/// - `version` — advertising gateway build version.
/// - `api` — gateway API version (`v1` today).
/// - `name` — human-readable display name.
/// - `fp` — unpadded base64url of `sha256(instance-id)`, present only once the
///   gateway has been paired at least once. Matches the `fp` computed by
///   `thinclaw_config::mdns_discovery::fingerprint_instance_id` on the server.
public struct DiscoveryTXTRecord: Sendable, Hashable {
    /// Advertising gateway build version, if present.
    public var version: String?
    /// Gateway API version (`v1`), if present.
    public var apiVersion: String?
    /// Human-readable display name from the TXT record, if present.
    public var name: String?
    /// Non-reversible fingerprint of the gateway instance id (`fp`), if the
    /// gateway has been paired. This is *not* the raw instance id and is used
    /// only as a locator hint to recognize a previously paired gateway.
    public var instanceFingerprint: String?

    public init(
        version: String? = nil,
        apiVersion: String? = nil,
        name: String? = nil,
        instanceFingerprint: String? = nil
    ) {
        self.version = version
        self.apiVersion = apiVersion
        self.name = name
        self.instanceFingerprint = instanceFingerprint
    }

    /// Parse from a decoded key/value dictionary (as produced from an
    /// `NWTXTRecord`). Empty values are treated as absent so a blank `name`
    /// does not shadow the Bonjour instance name.
    public init(dictionary: [String: String]) {
        func nonEmpty(_ key: String) -> String? {
            guard let value = dictionary[key]?.trimmingCharacters(in: .whitespacesAndNewlines),
                !value.isEmpty
            else { return nil }
            return value
        }
        self.init(
            version: nonEmpty("version"),
            apiVersion: nonEmpty("api"),
            name: nonEmpty("name"),
            instanceFingerprint: nonEmpty("fp"))
    }
}

#if canImport(CryptoKit)
    extension DiscoveryTXTRecord {
        /// Whether this record's `fp` matches the fingerprint the app derives
        /// from a pinned instance id via ``fingerprint(ofInstanceID:)``.
        ///
        /// A `false` result (no `fp`, or a mismatch) means the endpoint is
        /// *not* confirmed to be the paired gateway — but this is still only a
        /// hint. Even a `true` result does not authenticate the endpoint
        /// (D-X3): the SPKI pin and instance-id check at connection time remain
        /// mandatory.
        public func matchesInstance(id instanceID: String) -> Bool {
            guard let fingerprint = instanceFingerprint else { return false }
            return fingerprint == Self.fingerprint(ofInstanceID: instanceID)
        }

        /// Compute the discovery fingerprint of a gateway instance id: unpadded
        /// base64url of `sha256(instance-id UTF-8 bytes)`. Mirrors the server's
        /// `fingerprint_instance_id` so a locally pinned instance id can be
        /// matched against a rediscovered endpoint's advertised `fp` without
        /// the raw id ever crossing the wire.
        public static func fingerprint(ofInstanceID instanceID: String) -> String {
            SPKIFingerprint.base64url(spkiDER: Data(instanceID.utf8))
        }
    }
#endif
