import Foundation
import ThinClawAPI
import ThinClawAuth

/// How the one-time pairing credential was obtained, which drives the
/// `/api/devices/pair/complete` redemption path (D-P1): the 32-byte QR secret
/// or the short human-typable no-camera code — exactly one, never both.
public enum PairingRedemption: Sendable, Equatable, Codable {
    /// The base64url one-time secret carried in the QR payload (`sec`).
    case secret(String)
    /// The short human code typed in the no-camera fallback.
    case code(String)
}

/// The outcome of a `/api/devices/pair/complete` call.
public enum PairingResult: Sendable, Equatable {
    /// The gateway issued a device token immediately (auto-approve, D-P3).
    /// The credential is ready to persist.
    case paired(DeviceCredential)
    /// `device_pairing.require_confirm=true`: the request is queued for an
    /// operator to approve on an authenticated surface. No token yet.
    case pendingConfirmation(pairingID: String)
}

/// Why a pairing attempt failed, in terms the onboarding UI can turn into an
/// actionable, retryable message. Distinct from ``PairingPayloadError`` (which
/// covers only *parsing* the QR/link) — these cover the network round-trip.
public enum PairingError: Error, Equatable, Sendable {
    /// The pairing link / QR could not be parsed or had expired.
    case invalidPayload(PairingPayloadError)
    /// No candidate gateway URL survived the D-X2 connection policy (e.g. a
    /// `vpn-http` gateway advertised only refused plaintext endpoints).
    case noReachableEndpoint
    /// The gateway refused the secret/code: unknown, already used, or expired
    /// (HTTP 400). The operator should re-issue pairing.
    case rejectedCredential
    /// Too many pairing attempts (HTTP 429); back off and retry.
    case rateLimited(retryAfter: TimeInterval?)
    /// The gateway returned 5xx.
    case server(status: Int)
    /// The pinned SPKI did not match the live TLS identity (possible MITM).
    case pinMismatch
    /// The authenticated gateway returned a different installation identity
    /// than the QR payload claimed.
    case gatewayIdentityMismatch
    /// A network-level failure (unreachable, timed out).
    case transport(URLError.Code)
    /// Generating the Secure-Enclave / software device key failed.
    case keyGenerationFailed
    /// Persisting the issued credential to the Keychain failed.
    case credentialStorageFailed
    /// Anything else, preserved for diagnostics.
    case unexpected(status: Int)

    /// A short, human-readable, actionable description for the failed step.
    public var userMessage: String {
        switch self {
        case .invalidPayload(let underlying):
            switch underlying {
            case .notAPairingURL:
                return "That isn't a ThinClaw pairing link. Scan the QR code from "
                    + "your gateway's settings, or paste the thinclaw://pair link."
            case .malformedPayload:
                return "This pairing link is damaged. Re-open pairing on your "
                    + "gateway to get a fresh QR code."
            case .unsupportedVersion:
                return "This pairing link needs a newer app. Update ThinClaw and "
                    + "try again."
            case .expired:
                return "This pairing code has expired. Re-run pairing on your "
                    + "gateway (codes last 15 minutes)."
            case .noUsableURLs:
                return "This pairing link has no usable gateway address. "
                    + "Re-generate it from your gateway."
            }
        case .noReachableEndpoint:
            return "None of the gateway's addresses are reachable under the "
                + "security policy. Check you're on the same network or VPN."
        case .rejectedCredential:
            return "The gateway didn't recognize this code — it may have been "
                + "used already or expired. Start pairing again."
        case .rateLimited:
            return "Too many attempts. Wait a moment, then try pairing again."
        case .server(let status):
            return "The gateway hit an error (\(status)). Try again shortly."
        case .pinMismatch:
            return "The gateway's TLS identity didn't match what the QR promised. "
                + "For safety, pairing was stopped. Re-pair only from a trusted QR."
        case .gatewayIdentityMismatch:
            return "The gateway identity did not match the pairing code. For safety, pairing was stopped."
        case .transport:
            return "Couldn't reach the gateway. Check the connection and retry."
        case .keyGenerationFailed:
            return "Couldn't create this device's identity key. Try again."
        case .credentialStorageFailed:
            return "Paired, but couldn't save the credential securely. Try again."
        case .unexpected(let status):
            return "Unexpected response from the gateway (\(status)). Try again."
        }
    }
}

/// The seam the onboarding store calls to redeem a pairing credential. The live
/// implementation (``LivePairingService``) drives the pinned-TLS
/// `/api/devices/pair/complete` round-trip; tests inject a fake.
public protocol PairingService: Sendable {
    /// Redeem a pairing credential against the given gateway.
    ///
    /// - Parameters:
    ///   - payload: the parsed, validated pairing payload (URLs, pin, instance
    ///     id, name).
    ///   - redemption: the QR secret or the typed short code.
    ///   - deviceName: the human label to register (defaults to the device
    ///     name; editable on the confirm sheet).
    /// - Returns: ``PairingResult/paired`` with a ready-to-store credential, or
    ///   ``PairingResult/pendingConfirmation`` in require-confirm mode.
    func pair(
        payload: PairingPayload,
        redemption: PairingRedemption,
        deviceName: String
    ) async throws(PairingError) -> PairingResult
}
