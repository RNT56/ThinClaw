import Foundation
import Observation
import ThinClawAPI
import ThinClawAuth

/// The transport trust badge shown on the confirm sheet (D-X2): whether the
/// gateway will be reached over pinned/public-chain TLS, or plaintext
/// `vpn-http` that must be warned about.
public enum TransportBadge: Sendable, Equatable {
    /// Pinned SPKI or public-chain TLS — the default, trustworthy path.
    case pinnedTLS
    /// Plaintext HTTP over the tailnet (`vpn-http`, opt-in). The UI must warn.
    case vpnHTTPWarning
}

/// Pairing onboarding state machine (docs/MOBILE_SECURITY.md D-P1..D-X2).
///
/// Drives: scan/paste a `thinclaw://pair` link (or type a gateway URL + short
/// code), confirm the gateway identity + transport badge, redeem the credential
/// via ``PairingService``, persist it to the Keychain, and land in chat. Every
/// failure is a retryable, actionable ``Step/failed(message:)``.
///
/// UI-framework-free on purpose so the whole machine is exercised with a fake
/// ``PairingService`` and an ``InMemoryKeychain``.
@MainActor
@Observable
public final class OnboardingStore {
    public enum Step: Sendable, Equatable {
        /// Landing screen: offer scan / paste / manual code.
        case welcome
        /// The camera scanner is presented (iOS + camera permission only).
        case scanQR
        /// A payload parsed successfully; confirm before pairing. Carries the
        /// gateway name, instance id, and the transport trust badge (D-X2).
        case confirmGateway(name: String, instanceID: String, badge: TransportBadge)
        /// The `/pair/complete` round-trip is in flight.
        case pairing
        /// require_confirm mode: waiting for an operator to approve on an
        /// authenticated surface. Terminal for this session — no token yet.
        case pendingApproval(pairingID: String)
        /// A retryable failure with an actionable message.
        case failed(message: String)
        /// Paired; the credential is stored. The app flips to the tab shell.
        case done
    }

    public private(set) var step: Step = .welcome

    /// The editable device name shown on the confirm sheet. Seeded with the
    /// device's name by the composition root; the operator may override it.
    public var deviceName: String

    /// The payload awaiting confirmation, retained so `pair()` and retry don't
    /// need it re-supplied.
    public private(set) var pendingPayload: PairingPayload?
    /// The redemption path (QR secret or typed code) captured with the payload.
    public private(set) var pendingRedemption: PairingRedemption?

    private let pairingService: any PairingService
    private let keychain: any KeychainStoring
    /// Called after a credential is persisted, so the app can flip `isPaired`.
    private let onPaired: @MainActor (DeviceCredential) -> Void

    public init(
        pairingService: any PairingService,
        keychain: any KeychainStoring,
        deviceName: String,
        onPaired: @escaping @MainActor (DeviceCredential) -> Void = { _ in }
    ) {
        self.pairingService = pairingService
        self.keychain = keychain
        self.deviceName = deviceName
        self.onPaired = onPaired
    }

    // MARK: - Navigation

    /// Present the camera scanner (welcome → scanQR).
    public func startScanning() {
        step = .scanQR
    }

    /// Return to the landing screen and drop any pending payload.
    public func reset() {
        pendingPayload = nil
        pendingRedemption = nil
        step = .welcome
    }

    // MARK: - Payload entry

    /// Handle a scanned/opened pairing URL: parse, then advance to confirm.
    /// Any ``PairingPayloadError`` becomes an actionable failure.
    public func handleScanned(_ url: URL) {
        do {
            let payload = try PairingPayload.parse(from: url)
            present(payload: payload, redemption: .secret(payload.secret))
        } catch let error as PairingPayloadError {
            fail(.invalidPayload(error))
        } catch {
            fail(.invalidPayload(.malformedPayload))
        }
    }

    /// Manual no-camera path: the operator pastes the whole `thinclaw://pair`
    /// link. Identical to a scan.
    public func handlePastedLink(_ raw: String) {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: trimmed) else {
            fail(.invalidPayload(.notAPairingURL))
            return
        }
        handleScanned(url)
    }

    /// Manual no-camera path (no QR at all): the operator types a gateway base
    /// URL and the short human code. There is no SPKI pin or instance id to
    /// verify up front, so this is the `vpn-http`/trusted-network fallback and
    /// pairs immediately (no confirm sheet — the operator already typed the
    /// address deliberately).
    ///
    /// - Note: without a pinned fingerprint the D-X2 policy still refuses
    ///   plaintext to LAN/public, so an `https://` or tailnet address is
    ///   required for the connection to be attempted.
    public func pairWithManualCode(gatewayURL raw: String, code: String) async {
        let trimmedURL = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        let trimmedCode = code.trimmingCharacters(in: .whitespacesAndNewlines)
        guard
            let url = URL(string: trimmedURL),
            let scheme = url.scheme?.lowercased(),
            scheme == "http" || scheme == "https",
            url.host != nil
        else {
            fail(.invalidPayload(.noUsableURLs))
            return
        }
        guard !trimmedCode.isEmpty else {
            fail(.rejectedCredential)
            return
        }
        // Synthesize a fingerprint-less payload for the shared pair() path.
        let payload = PairingPayload(
            version: PairingPayload.supportedVersion,
            urls: [url],
            fingerprint: nil,
            installationID: "",
            name: url.host ?? "gateway",
            secret: "",
            expiresAt: .distantFuture)
        pendingPayload = payload
        pendingRedemption = .code(trimmedCode)
        await runPairing(payload: payload, redemption: .code(trimmedCode))
    }

    private func present(payload: PairingPayload, redemption: PairingRedemption) {
        pendingPayload = payload
        pendingRedemption = redemption
        step = .confirmGateway(
            name: payload.name,
            instanceID: payload.installationID,
            badge: Self.badge(for: payload))
    }

    /// D-X2 badge for the confirm sheet: `vpn-http` warning only when there is
    /// no pin *and* the sole permitted path to the preferred endpoint is
    /// plaintext; otherwise pinned/public-chain TLS.
    static func badge(for payload: PairingPayload) -> TransportBadge {
        if payload.fingerprint != nil { return .pinnedTLS }
        // No pin: inspect the first candidate URL. https ⇒ public-chain TLS;
        // an allowed plaintext endpoint ⇒ the badged vpn-http path.
        for url in payload.urls {
            switch ConnectionPolicy.evaluate(url: url, hasPin: false, allowLoopbackHTTP: false) {
            case .allowedSecure: return .pinnedTLS
            case .allowedInsecure: return .vpnHTTPWarning
            case .refused: continue
            }
        }
        // Nothing permitted; treat as the warned path so the sheet is honest.
        return .vpnHTTPWarning
    }

    // MARK: - Pairing

    /// Confirm the gateway and redeem the credential (confirmGateway → pairing
    /// → done / pendingApproval / failed).
    public func confirmAndPair() async {
        guard let payload = pendingPayload, let redemption = pendingRedemption else {
            fail(.unexpected(status: 0))
            return
        }
        await runPairing(payload: payload, redemption: redemption)
    }

    /// Retry the last attempt after a failure, reusing the captured payload.
    public func retry() async {
        guard let payload = pendingPayload, let redemption = pendingRedemption else {
            reset()
            return
        }
        await runPairing(payload: payload, redemption: redemption)
    }

    private func runPairing(payload: PairingPayload, redemption: PairingRedemption) async {
        step = .pairing
        let name = deviceName.trimmingCharacters(in: .whitespacesAndNewlines)
        let effectiveName = name.isEmpty ? "iPhone" : name
        do {
            let result = try await pairingService.pair(
                payload: payload, redemption: redemption, deviceName: effectiveName)
            switch result {
            case .paired(let credential):
                do {
                    try credential.save(to: keychain)
                } catch {
                    fail(.credentialStorageFailed)
                    return
                }
                onPaired(credential)
                step = .done
            case .pendingConfirmation(let pairingID):
                step = .pendingApproval(pairingID: pairingID)
            }
        } catch let error as PairingError {
            fail(error)
        } catch {
            fail(.unexpected(status: 0))
        }
    }

    private func fail(_ error: PairingError) {
        step = .failed(message: error.userMessage)
    }
}
