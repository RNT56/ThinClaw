import SwiftUI
import ThinClawDesign

/// App settings: gateway connection details (biometric-gated reveal), this
/// device's identity, notification preferences, privacy toggles.
@MainActor
@Observable
public final class SettingsStore {
    public private(set) var gatewayName: String = "—"
    public private(set) var deviceName: String = "—"
    public var enhancedProtection: Bool = false

    public init() {}

    /// M5: GET /api/devices/me, unpair (DELETE), token rotation,
    /// notification preview preferences.
    public func refresh() async {}
}

public struct SettingsScreen: View {
    @State private var store: SettingsStore

    public init(store: SettingsStore = SettingsStore()) {
        self._store = State(initialValue: store)
    }

    public var body: some View {
        Form {
            Section("Gateway") {
                LabeledContent("Instance", value: store.gatewayName)
                LabeledContent("This device", value: store.deviceName)
            }
            Section {
                Toggle("Enhanced protection", isOn: $store.enhancedProtection)
            } footer: {
                Text(
                    "Upgrades the local cache to full file protection. "
                        + "Widgets stop refreshing while the device is locked.")
            }
        }
        .navigationTitle("Settings")
        .task { await store.refresh() }
    }
}
