#if canImport(Security) && canImport(CryptoKit)
    import Testing

    @testable import FeatureOnboarding

    @Suite("LivePairingService gateway identity")
    struct LivePairingIdentityTests {
        @Test("matching QR identity succeeds")
        func matchingQR() throws {
            try LivePairingService.validateGatewayIdentity(
                payloadInstallationID: "gateway-a",
                responseInstallationID: "gateway-a")
        }

        @Test("mismatched QR identity fails closed")
        func mismatchedQR() {
            #expect(throws: PairingError.gatewayIdentityMismatch) {
                try LivePairingService.validateGatewayIdentity(
                    payloadInstallationID: "gateway-a",
                    responseInstallationID: "gateway-b")
            }
        }

        @Test("manual code adopts the authoritative server identity")
        func manualCodeIdentity() throws {
            try LivePairingService.validateGatewayIdentity(
                payloadInstallationID: "",
                responseInstallationID: "gateway-server")
        }
    }
#endif
