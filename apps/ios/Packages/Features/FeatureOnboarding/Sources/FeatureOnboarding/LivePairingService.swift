import Foundation
import ThinClawAPI
import ThinClawAuth

#if canImport(Security) && canImport(CryptoKit)
    import OpenAPIRuntime
    import OpenAPIURLSession

    /// Production ``PairingService``: redeems a pairing credential over a
    /// pinned-TLS connection to `POST /api/devices/pair/complete`
    /// (docs/MOBILE_SECURITY.md D-P1/D-P2/D-X2).
    ///
    /// Flow:
    ///   1. Filter the payload's candidate URLs through ``ConnectionPolicy``
    ///      (D-X2); refuse if none survive.
    ///   2. Generate the device P-256 identity key (Secure Enclave, software
    ///      fallback on the simulator) and submit its SPKI (D-P2, stored not
    ///      yet enforced).
    ///   3. POST the one-time secret **or** the short code (never both) to the
    ///      *public* complete endpoint over a ``PinnedSessionDelegate`` session
    ///      so the pin is verified on the very first byte (no TOFU window).
    ///   4. On 200, assemble a ``DeviceCredential``; on 202, report
    ///      pending-confirmation.
    public struct LivePairingService: PairingService {
        /// Client platform string sent as `platform` in the pair body.
        private let platform: String
        /// Test hook to force the software key path (skips the enclave).
        private let forceSoftwareKey: Bool
        /// DEBUG-only escape hatch to allow plaintext loopback during dev.
        private let allowLoopbackHTTP: Bool
        /// Injected clock, so `pairedAt` is deterministic under test.
        private let now: @Sendable () -> Date

        public init(
            platform: String = "ios",
            forceSoftwareKey: Bool = false,
            allowLoopbackHTTP: Bool = false,
            now: @escaping @Sendable () -> Date = { Date() }
        ) {
            self.platform = platform
            self.forceSoftwareKey = forceSoftwareKey
            self.allowLoopbackHTTP = allowLoopbackHTTP
            self.now = now
        }

        public func pair(
            payload: PairingPayload,
            redemption: PairingRedemption,
            deviceName: String
        ) async throws(PairingError) -> PairingResult {
            let hasPin = payload.fingerprint != nil
            // D-X2: keep only endpoints the policy permits, in preference order.
            let allowed = payload.urls.filter { url in
                switch ConnectionPolicy.evaluate(
                    url: url, hasPin: hasPin, allowLoopbackHTTP: allowLoopbackHTTP)
                {
                case .allowedSecure, .allowedInsecure: return true
                case .refused: return false
                }
            }
            guard let baseURL = allowed.first else {
                throw PairingError.noReachableEndpoint
            }

            // D-P2: device identity key; its SPKI rides in the pair body.
            let keyHandle: DeviceKeyPair.Handle
            do {
                keyHandle = try DeviceKeyPair.generate(forceSoftware: forceSoftwareKey)
            } catch {
                throw PairingError.keyGenerationFailed
            }

            let (secret, code): (String?, String?)
            switch redemption {
            case .secret(let value): (secret, code) = (value, nil)
            case .code(let value): (secret, code) = (nil, value)
            }

            let requestBody = Components.Schemas.PairCompleteRequest(
                code: code,
                name: deviceName,
                platform: platform,
                pubkey: keyHandle.spkiBase64,
                secret: secret)

            // Pinned session -> transport -> token-less client. The complete
            // endpoint is public, so this client carries no bearer middleware.
            let delegate = PinnedSessionDelegate(pinnedFingerprint: payload.fingerprint)
            let transport = URLSessionTransport(
                configuration: .init(session: delegate.makeSession()))
            let client = Client(serverURL: baseURL, transport: transport)

            let output: Operations.DevicesPairCompleteHandler.Output
            do {
                output = try await client.devicesPairCompleteHandler(body: .json(requestBody))
            } catch {
                throw Self.mapCallError(error)
            }

            switch output {
            case .ok(let ok):
                let response: Components.Schemas.PairCompleteResponse
                do {
                    response = try ok.body.json
                } catch {
                    throw PairingError.unexpected(status: 200)
                }
                guard DeviceToken.isWellFormed(response.token) else {
                    throw PairingError.unexpected(status: 200)
                }
                try Self.validateGatewayIdentity(
                    payloadInstallationID: payload.installationID,
                    responseInstallationID: response.gatewayInstance)
                let credential = DeviceCredential(
                    installationID: response.gatewayInstance,
                    deviceID: response.deviceId,
                    deviceToken: response.token,
                    // Persist only the policy-allowed endpoints (D-X2), never
                    // the raw payload list: a plaintext LAN URL must never be
                    // stored where a later session builder could pick it up and
                    // send the token in the clear.
                    gatewayURLs: allowed,
                    serverFingerprint: payload.fingerprint,
                    gatewayName: payload.name,
                    pairedAt: now())
                return .paired(credential)

            case .accepted(let accepted):
                let pending: Components.Schemas.PairPendingConfirmResponse
                do {
                    pending = try accepted.body.json
                } catch {
                    throw PairingError.unexpected(status: 202)
                }
                return .pendingConfirmation(pairingID: pending.pairingId)

            case .badRequest:
                throw PairingError.rejectedCredential
            case .tooManyRequests:
                throw PairingError.rateLimited(retryAfter: nil)
            case .undocumented(let status, _):
                throw Self.mapStatus(status)
            }
        }

        /// Map an error thrown by the generated call (transport, pin failures,
        /// or an `APIError` surfaced by middleware) into a ``PairingError``.
        static func mapCallError(_ error: any Error) -> PairingError {
            switch APIError.from(error) {
            case .unauthorized: return .rejectedCredential
            case .forbidden: return .rejectedCredential
            case .rateLimited(let retryAfter): return .rateLimited(retryAfter: retryAfter)
            case .server(let status): return .server(status: status)
            case .pinMismatch: return .pinMismatch
            case .transport(let code):
                // A cancelled TLS challenge (pin mismatch) surfaces as a
                // cancellation from URLSession; treat it as a pin failure so the
                // operator isn't told to "check the connection".
                if code == .cancelled || code == .secureConnectionFailed
                    || code == .serverCertificateUntrusted
                {
                    return .pinMismatch
                }
                return .transport(code)
            case .notPaired: return .unexpected(status: 0)
            case .unexpected(let status): return mapStatus(status)
            }
        }

        static func mapStatus(_ status: Int) -> PairingError {
            switch status {
            case 400: return .rejectedCredential
            case 401, 403: return .rejectedCredential
            case 429: return .rateLimited(retryAfter: nil)
            case 500...599: return .server(status: status)
            default: return .unexpected(status: status)
            }
        }

        /// QR pairing binds the locator identity to the authenticated response.
        /// Manual-code pairing intentionally supplies an empty locator identity
        /// and therefore always adopts the server-returned value.
        static func validateGatewayIdentity(
            payloadInstallationID: String,
            responseInstallationID: String
        ) throws(PairingError) {
            guard
                payloadInstallationID.isEmpty
                    || payloadInstallationID == responseInstallationID
            else { throw .gatewayIdentityMismatch }
        }
    }
#endif
