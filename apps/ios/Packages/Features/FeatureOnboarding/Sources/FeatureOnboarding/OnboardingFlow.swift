import SwiftUI
import ThinClawDesign

/// Root onboarding view: composes the ``OnboardingStore`` state machine into the
/// scan / paste / confirm / pairing screens. Liquid Glass styling comes from
/// the OS materials via ThinClawDesign tokens.
public struct OnboardingFlow: View {
    @State private var store: OnboardingStore
    @State private var showsScanner = false
    @State private var showsManualEntry = false

    public init(store: OnboardingStore) {
        self._store = State(initialValue: store)
    }

    public var body: some View {
        VStack(spacing: ThinClawSpacing.lg) {
            switch store.step {
            case .welcome, .scanQR:
                welcome
            case .confirmGateway(let name, let instanceID, let badge):
                ConfirmGatewaySheet(
                    name: name,
                    instanceID: instanceID,
                    badge: badge,
                    deviceName: $store.deviceName,
                    onPair: { await store.confirmAndPair() },
                    onCancel: { store.reset() })
            case .pairing:
                ProgressView("Pairing…")
                    .controlSize(.large)
            case .pendingApproval:
                pendingApproval
            case .failed(let message):
                failure(message)
            case .done:
                Label("Paired", systemImage: "checkmark.seal.fill")
                    .font(.title2)
                    .foregroundStyle(.green)
            }
        }
        .padding(ThinClawSpacing.xl)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .sheet(isPresented: $showsScanner) {
            QRScannerSheet { url in
                showsScanner = false
                store.handleScanned(url)
            } onCancel: {
                showsScanner = false
            }
        }
        .sheet(isPresented: $showsManualEntry) {
            ManualEntrySheet(
                onLink: { link in
                    showsManualEntry = false
                    store.handlePastedLink(link)
                },
                onCode: { url, code in
                    showsManualEntry = false
                    await store.pairWithManualCode(gatewayURL: url, code: code)
                },
                onCancel: { showsManualEntry = false })
        }
    }

    private var welcome: some View {
        ContentUnavailableView {
            Label("Pair with your ThinClaw", systemImage: "qrcode.viewfinder")
        } description: {
            Text(
                "Run `thinclaw devices pair` or open the gateway settings, "
                    + "then scan the QR code it shows.")
        } actions: {
            VStack(spacing: ThinClawSpacing.md) {
                if QRScannerSheet.isSupported {
                    Button {
                        store.startScanning()
                        showsScanner = true
                    } label: {
                        Label("Scan QR code", systemImage: "qrcode.viewfinder")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.glassProminent)
                }
                Button {
                    showsManualEntry = true
                } label: {
                    Label("Enter link or code", systemImage: "keyboard")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.glass)
            }
            .frame(maxWidth: 320)
        }
    }

    private var pendingApproval: some View {
        VStack(spacing: ThinClawSpacing.md) {
            Image(systemName: "hourglass")
                .font(.largeTitle)
                .foregroundStyle(.secondary)
            Text("Waiting for approval")
                .font(.title3.bold())
            Text(
                "Your gateway requires an operator to approve new devices. "
                    + "Approve this device from the ThinClaw web UI or CLI, then "
                    + "reopen the app."
            )
            .font(ThinClawTypography.caption)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
            Button("Start over") { store.reset() }
                .buttonStyle(.glass)
        }
    }

    private func failure(_ message: String) -> some View {
        VStack(spacing: ThinClawSpacing.md) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.largeTitle)
                .foregroundStyle(.orange)
            Text(message)
                .font(ThinClawTypography.body)
                .multilineTextAlignment(.center)
            HStack(spacing: ThinClawSpacing.md) {
                Button("Start over") { store.reset() }
                    .buttonStyle(.glass)
                Button("Try again") {
                    Task { await store.retry() }
                }
                .buttonStyle(.glassProminent)
            }
        }
    }
}

/// The confirm sheet shown after a payload parses: gateway identity, the D-X2
/// transport badge, and the editable device name.
struct ConfirmGatewaySheet: View {
    let name: String
    let instanceID: String
    let badge: TransportBadge
    @Binding var deviceName: String
    let onPair: () async -> Void
    let onCancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: ThinClawSpacing.lg) {
            VStack(alignment: .leading, spacing: ThinClawSpacing.xs) {
                Text("Pair with")
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
                Text(name)
                    .font(.title2.bold())
                if !instanceID.isEmpty {
                    Text(instanceID)
                        .font(ThinClawTypography.mono)
                        .foregroundStyle(.secondary)
                }
            }

            transportBadge

            VStack(alignment: .leading, spacing: ThinClawSpacing.xs) {
                Text("This device's name")
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
                TextField("Device name", text: $deviceName)
                    .textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.words)
            }

            HStack(spacing: ThinClawSpacing.md) {
                Button("Cancel", action: onCancel)
                    .buttonStyle(.glass)
                Button("Pair") { Task { await onPair() } }
                    .buttonStyle(.glassProminent)
                    .disabled(deviceName.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
        .padding(ThinClawSpacing.xl)
    }

    @ViewBuilder private var transportBadge: some View {
        switch badge {
        case .pinnedTLS:
            Label("Encrypted, pinned connection", systemImage: "lock.fill")
                .font(ThinClawTypography.caption)
                .foregroundStyle(.green)
        case .vpnHTTPWarning:
            Label {
                Text(
                    "Unencrypted VPN connection (vpn-http). Only continue on a "
                        + "trusted tailnet.")
            } icon: {
                Image(systemName: "exclamationmark.shield.fill")
            }
            .font(ThinClawTypography.caption)
            .foregroundStyle(.orange)
        }
    }
}

/// No-camera manual entry: paste a full `thinclaw://pair` link, or type a
/// gateway URL plus the short human code. The simulator uses this path.
struct ManualEntrySheet: View {
    let onLink: (String) -> Void
    let onCode: (String, String) async -> Void
    let onCancel: () -> Void

    @State private var link = ""
    @State private var gatewayURL = ""
    @State private var code = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("Paste pairing link") {
                    TextField("thinclaw://pair?d=…", text: $link)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button("Use link") { onLink(link) }
                        .disabled(link.trimmingCharacters(in: .whitespaces).isEmpty)
                }
                Section("Or gateway address + code") {
                    TextField("https://gateway.local:3443", text: $gatewayURL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                    TextField("Short code", text: $code)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button("Pair with code") {
                        Task { await onCode(gatewayURL, code) }
                    }
                    .disabled(
                        gatewayURL.trimmingCharacters(in: .whitespaces).isEmpty
                            || code.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            .navigationTitle("Enter pairing details")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel", action: onCancel)
                }
            }
        }
    }
}
