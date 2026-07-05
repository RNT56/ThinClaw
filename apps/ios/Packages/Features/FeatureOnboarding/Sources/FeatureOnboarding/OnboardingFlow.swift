import SwiftUI
import ThinClawAuth
import ThinClawDesign

/// Pairing onboarding: scan the QR from the gateway (or pick a
/// Bonjour-discovered instance and type the short code), verify the pinned
/// TLS identity, store the device credential, land in chat.
@MainActor
@Observable
public final class OnboardingStore {
    public enum Step: Hashable, Sendable {
        case welcome
        case scanQR
        case confirmGateway(name: String, instanceID: String)
        case pairing
        case failed(message: String)
        case done
    }

    public private(set) var step: Step = .welcome

    public init() {}

    /// M1: parse `PairingPayload`, open the pinned-TLS connection
    /// (docs/MOBILE_SECURITY.md D-X2 policy matrix), generate the
    /// Secure-Enclave keypair, POST /api/devices/pair/complete, store the
    /// `tcd_` token in the shared Keychain.
    public func handleScanned(_ url: URL) async {}
}

public struct OnboardingFlow: View {
    @State private var store: OnboardingStore

    public init(store: OnboardingStore = OnboardingStore()) {
        self._store = State(initialValue: store)
    }

    public var body: some View {
        VStack(spacing: ThinClawSpacing.lg) {
            switch store.step {
            case .welcome:
                ContentUnavailableView {
                    Label("Pair with your ThinClaw", systemImage: "qrcode.viewfinder")
                } description: {
                    Text(
                        "Run `thinclaw devices pair` or open the gateway settings, "
                            + "then scan the QR code it shows.")
                } actions: {
                    Button("Scan QR code") {}
                        .buttonStyle(.glassProminent)
                }
            case .scanQR, .confirmGateway, .pairing:
                ProgressView("Pairing…")
            case .failed(let message):
                Label(message, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
            case .done:
                Label("Paired", systemImage: "checkmark.seal")
            }
        }
        .padding(ThinClawSpacing.xl)
    }
}
