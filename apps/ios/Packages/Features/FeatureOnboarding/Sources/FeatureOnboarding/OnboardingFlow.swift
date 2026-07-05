import SwiftUI
import ThinClawAuth
import ThinClawDesign

/// Root onboarding view: composes the ``OnboardingStore`` state machine into the
/// scan / paste / confirm / pairing screens. Liquid Glass styling comes from
/// the OS materials via ThinClawDesign tokens.
public struct OnboardingFlow: View {
    @State private var store: OnboardingStore
    @State private var discovery: DiscoveryStore
    @State private var showsScanner = false
    @State private var showsManualEntry = false
    /// Gateway URL to seed the manual-entry form with when it opens, e.g. from a
    /// discovered gateway. A locator hint only — the QR secret or short code is
    /// still required (D-X3).
    @State private var manualEntryPrefill = ""

    /// - Parameters:
    ///   - store: the pairing state machine (from the composition root).
    ///   - discovery: the Bonjour discovery store. Defaults to a live
    ///     ``DiscoveryStore`` backed by `NWBrowser`; tests inject a fake.
    public init(store: OnboardingStore, discovery: DiscoveryStore = DiscoveryStore()) {
        self._store = State(initialValue: store)
        self._discovery = State(initialValue: discovery)
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
                prefilledGatewayURL: manualEntryPrefill,
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
        .onDisappear { discovery.stop() }
    }

    /// Open manual entry, optionally seeding the gateway-URL field (e.g. from a
    /// discovered candidate). Discovery is paused while the sheet is up.
    private func presentManualEntry(prefill: String = "") {
        manualEntryPrefill = prefill
        discovery.stop()
        showsManualEntry = true
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
                    presentManualEntry()
                } label: {
                    Label("Enter link or code", systemImage: "keyboard")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.glass)

                DiscoverySection(discovery: discovery) { gateway in
                    presentManualEntry(prefill: gateway.suggestedBaseURL?.absoluteString ?? "")
                }
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
    /// Seed for the gateway-URL field, e.g. the base URL of a discovered
    /// gateway. Empty when the user opened manual entry directly.
    var prefilledGatewayURL: String = ""
    let onLink: (String) -> Void
    let onCode: (String, String) async -> Void
    let onCancel: () -> Void

    @State private var link = ""
    @State private var gatewayURL = ""
    @State private var code = ""

    var body: some View {
        NavigationStack {
            Form {
                if !prefilledGatewayURL.isEmpty {
                    Section {
                        Label {
                            Text(
                                "Found this gateway on your network. Discovery "
                                    + "only locates it — finish pairing with the "
                                    + "QR link or short code so the app can "
                                    + "verify it.")
                        } icon: {
                            Image(systemName: "bonjour")
                        }
                        .font(ThinClawTypography.caption)
                        .foregroundStyle(.secondary)
                    }
                }
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
        .onAppear {
            if gatewayURL.isEmpty { gatewayURL = prefilledGatewayURL }
        }
    }
}

/// The "Discover on this network" affordance shown on the welcome step
/// (milestone B3). Lists gateways found via Bonjour and, on tap, hands the
/// candidate back so the caller can pre-fill the pairing form.
///
/// **Locator only.** Tapping a row never sends anything to the gateway; it only
/// suggests a URL to type into the manual-entry sheet. Pairing still needs the
/// QR secret or short code, and the connection still verifies the pinned SPKI +
/// instance id (docs/MOBILE_SECURITY.md D-X3 / T11). The copy makes that clear.
struct DiscoverySection: View {
    let discovery: DiscoveryStore
    let onSelect: (DiscoveredGateway) -> Void

    var body: some View {
        VStack(spacing: ThinClawSpacing.sm) {
            if !discovery.isBrowsing {
                Button {
                    discovery.start()
                } label: {
                    Label("Discover on this network", systemImage: "bonjour")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.glass)
            } else {
                browsingBody
            }
        }
    }

    @ViewBuilder private var browsingBody: some View {
        HStack(spacing: ThinClawSpacing.xs) {
            ProgressView().controlSize(.small)
            Text("Looking for gateways…")
                .font(ThinClawTypography.caption)
                .foregroundStyle(.secondary)
            Spacer()
            Button("Stop") { discovery.stop() }
                .font(ThinClawTypography.caption)
                .buttonStyle(.plain)
        }

        if discovery.gateways.isEmpty {
            Text(
                "No gateways found yet. Make sure discovery is enabled on your "
                    + "gateway, or pair with the QR code above."
            )
            .font(ThinClawTypography.caption)
            .foregroundStyle(.secondary)
            .multilineTextAlignment(.center)
        } else {
            ForEach(discovery.gateways) { gateway in
                Button {
                    onSelect(gateway)
                } label: {
                    DiscoveredGatewayRow(gateway: gateway)
                }
                .buttonStyle(.glass)
            }
        }

        Text("Discovery just finds candidates — you still confirm with the QR code or short code.")
            .font(ThinClawTypography.caption)
            .foregroundStyle(.tertiary)
            .multilineTextAlignment(.center)
    }
}

/// One discovered-gateway row: display name plus its resolved address (or a
/// "resolving" hint while the endpoint has no host/port yet).
private struct DiscoveredGatewayRow: View {
    let gateway: DiscoveredGateway

    var body: some View {
        HStack(spacing: ThinClawSpacing.sm) {
            Image(systemName: "server.rack")
                .foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(gateway.displayName)
                    .font(ThinClawTypography.body)
                    .foregroundStyle(.primary)
                Text(subtitle)
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Image(systemName: "chevron.right")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var subtitle: String {
        if let host = gateway.host, let port = gateway.port {
            return "\(host):\(port)"
        }
        return "Resolving…"
    }
}
