import SwiftUI
import ThinClawCore
import ThinClawDesign

#if canImport(UIKit)
    import UIKit
    import UserNotifications
#endif

/// In-app Settings (docs/MOBILE_APP.md M5): this device's identity + scopes, the
/// paired watch (companion) with a Revoke action, unpair, per-category
/// notification preview preferences (D-N3), the paired-gateway connection
/// summary with a Face-ID-gated URL/pin reveal (D-K3), and the enhanced-cache
/// protection toggle.
///
/// The store (`ThinClawCore.SettingsStore`) is UI-free and macOS-tested; this
/// view is the thin iOS shell. It requires a paired store, so the app presents
/// it only while paired.
public struct SettingsScreen: View {
    @State private var store: SettingsStore
    @State private var notificationAuthDenied = false
    @State private var showsUnpairOptions = false
    @Environment(\.scenePhase) private var scenePhase

    public init(store: SettingsStore) {
        self._store = State(initialValue: store)
    }

    public var body: some View {
        Form {
            thisDeviceSection
            companionsSection
            notificationsSection
            connectionSection
            protectionSection
            unpairSection
        }
        .navigationTitle("Settings")
        .task { await store.start() }
        .refreshable { await store.refreshDevices() }
        .onChange(of: scenePhase) { _, phase in
            // Re-hide the sensitive connection detail whenever the app leaves the
            // foreground so a resumed session re-gates it (D-K3).
            if phase != .active { store.hideConnectionDetail() }
        }
        .confirmationDialog(
            "Remove this gateway?",
            isPresented: $showsUnpairOptions,
            titleVisibility: .visible
        ) {
            Button("Revoke device and erase local data", role: .destructive) {
                Task { await store.unpair() }
            }
            Button("Erase local data only", role: .destructive) {
                Task { await store.removeLocalDataOnly() }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "Use local-only removal when the gateway is unreachable. An administrator must revoke the orphaned device record later."
            )
        }
    }

    // MARK: - This device

    @ViewBuilder private var thisDeviceSection: some View {
        Section("This device") {
            if let device = store.thisDevice {
                LabeledContent("Name", value: device.name)
                LabeledContent("Platform", value: device.platform)
                LabeledContent("Token", value: device.tokenPrefix + "…")
                LabeledContent("Last seen", value: device.lastSeenAt)
                scopeChips(device.scopes)
            } else if store.isLoadingDevices {
                ProgressView()
            } else if let error = store.deviceError {
                Label(error, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
            }
        }
    }

    @ViewBuilder private func scopeChips(_ scopes: [DeviceScopeTag]) -> some View {
        AdaptiveFlowLayout(spacing: ThinClawSpacing.xs) {
            ForEach(Array(scopes.enumerated()), id: \.offset) { _, scope in
                Text(scope.label)
                    .font(.caption2)
                    .padding(.horizontal, ThinClawSpacing.xs)
                    .padding(.vertical, 2)
                    .background(.thinMaterial, in: Capsule())
            }
        }
    }

    // MARK: - Companions (the watch)

    @ViewBuilder private var companionsSection: some View {
        Section {
            if store.companions.isEmpty {
                Text("No paired watch.")
                    .foregroundStyle(.secondary)
            } else {
                ForEach(store.companions) { companion in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(companion.name)
                            Text(companion.platform)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        }
                        Spacer()
                        Button("Revoke", role: .destructive) {
                            Task { await store.revokeCompanion(id: companion.id) }
                        }
                        .buttonStyle(.borderless)
                    }
                }
            }
        } header: {
            Text("Paired devices")
        } footer: {
            Text("Revoking a watch signs it out immediately and clears its access.")
        }
    }

    // MARK: - Notification previews (D-N3)

    @ViewBuilder private var notificationsSection: some View {
        Section {
            ForEach(NotificationCategory.allCases, id: \.self) { category in
                Picker(
                    Self.categoryTitle(category),
                    selection: Binding(
                        get: { store.notificationPreferences.mode(for: category) },
                        set: { store.setPreviewMode($0, for: category) })
                ) {
                    ForEach(NotificationPreferences.allowedModes(for: category), id: \.self) { mode in
                        Text(Self.previewModeTitle(mode)).tag(mode)
                    }
                }
            }
        } header: {
            Text("Notification previews")
        } footer: {
            if notificationAuthDenied {
                VStack(alignment: .leading, spacing: ThinClawSpacing.xs) {
                    Text("Notifications are turned off for ThinClaw.")
                    Button("Open Settings") { openSystemSettings() }
                }
            } else {
                Text("Controls how much content a notification reveals before you unlock.")
            }
        }
        .task { await refreshNotificationAuthorization() }
    }

    // MARK: - Connection (D-K3 gated reveal)

    @ViewBuilder private var connectionSection: some View {
        Section("Gateway") {
            LabeledContent("Name", value: store.connectionInfo.gatewayName)
            LabeledContent("Instance", value: store.connectionInfo.instanceID)
            LabeledContent("Status", value: store.connectionInfo.reachability.label)
            if let detail = store.connectionInfo.revealedDetail {
                LabeledContent("URL", value: detail.gatewayURL)
                if let pin = detail.pinnedFingerprint {
                    LabeledContent("Pin", value: pin)
                }
                Button("Hide details") { store.hideConnectionDetail() }
            } else {
                Button {
                    Task { await store.revealConnectionDetail() }
                } label: {
                    Label("Show connection details", systemImage: "faceid")
                }
            }
        }
    }

    // MARK: - Enhanced protection

    @ViewBuilder private var protectionSection: some View {
        Section {
            Toggle(
                "Enhanced protection",
                isOn: Binding(
                    get: { store.enhancedProtection },
                    set: { enabled in Task { await store.setEnhancedProtection(enabled) } }))
        } footer: {
            Text(
                "Encrypts the local transcript cache with full file protection. "
                    + "Notifications and widgets stop refreshing while the device is locked.")
        }
    }

    // MARK: - Unpair

    @ViewBuilder private var unpairSection: some View {
        Section {
            Button("Unpair this device", role: .destructive) {
                showsUnpairOptions = true
            }
        } footer: {
            Text("Revokes this device when reachable and erases its protected local data.")
        }
    }

    // MARK: - Notification authorization

    private func refreshNotificationAuthorization() async {
        #if canImport(UIKit)
            let center = UNUserNotificationCenter.current()
            let settings = await center.notificationSettings()
            notificationAuthDenied = settings.authorizationStatus == .denied
        #endif
    }

    private func openSystemSettings() {
        #if canImport(UIKit)
            if let url = URL(string: UIApplication.openSettingsURLString) {
                UIApplication.shared.open(url)
            }
        #endif
    }

    // MARK: - Labels

    static func categoryTitle(_ category: NotificationCategory) -> String {
        switch category {
        case .message: return "Messages"
        case .approval: return "Approvals"
        case .job: return "Jobs"
        }
    }

    static func previewModeTitle(_ mode: PreviewMode) -> String {
        switch mode {
        case .always: return "Always"
        case .whenUnlocked: return "When unlocked"
        case .never: return "Never"
        case .appOnly: return "App only"
        }
    }
}
