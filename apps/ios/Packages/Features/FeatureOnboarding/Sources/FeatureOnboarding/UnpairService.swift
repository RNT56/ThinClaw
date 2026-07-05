import Foundation
import ThinClawAuth

#if canImport(Security) && canImport(CryptoKit)
    import OpenAPIURLSession
    import ThinClawAPI

    /// Best-effort self-revoke used by the app's Unpair seam. Kept here so the
    /// App target never imports the OpenAPI runtime directly (FeatureOnboarding
    /// already owns that dependency for pairing).
    public enum UnpairService {
        /// Attempt `POST /api/devices/{id}/revoke` for this credential over its
        /// pinned session. All failures are swallowed — the caller erases the
        /// local credential regardless, so the device stops trusting the
        /// gateway even if the network call can't land.
        ///
        /// - Returns: `true` if the gateway acknowledged the revoke, `false`
        ///   otherwise (no device id, unreachable, or an error status).
        @discardableResult
        public static func revoke(_ credential: DeviceCredential) async -> Bool {
            guard let deviceID = credential.deviceID,
                let baseURL = credential.preferredBaseURL
            else { return false }
            let delegate = PinnedSessionDelegate(pinnedFingerprint: credential.serverFingerprint)
            let transport = URLSessionTransport(
                configuration: .init(session: delegate.makeSession()))
            let token = credential.deviceToken
            let client = GatewayClient.make(
                baseURL: baseURL, token: { token }, transport: transport)
            do {
                _ = try await client.devicesRevokeHandler(path: .init(id: deviceID))
                return true
            } catch {
                return false
            }
        }
    }
#else
    /// No Security/CryptoKit (non-Apple): revoke is a no-op.
    public enum UnpairService {
        @discardableResult
        public static func revoke(_ credential: DeviceCredential) async -> Bool { false }
    }
#endif
