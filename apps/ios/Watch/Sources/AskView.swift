import SwiftUI
import ThinClawSnapshotKit

#if canImport(WatchKit)
    import WatchKit
#endif

/// Dictated quick prompt.
///
/// The watch `TextField` presents the standard dictation / scribble affordance;
/// on submit the prompt goes through ``WatchStore`` → ``WatchGatewayProxy``
/// (`quickAsk`, forwarding the watch's own token). The *answer* is not shown
/// inline — it arrives later as a push or a refreshed snapshot — so this screen
/// only confirms delivery: "Sent", "Will send when reachable", or the failure.
struct AskView: View {
    @State var store: WatchStore

    @State private var prompt: String = ""
    @State private var isSending = false
    @State private var receipt: QuickAskReceipt?

    private var trimmed: String {
        prompt.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        Form {
            Section {
                TextField("Ask ThinClaw…", text: $prompt, axis: .vertical)
                    .lineLimit(1...4)
                    .textInputAutocapitalization(.sentences)
                    .submitLabel(.send)
                    .onSubmit(send)
            } footer: {
                Text("Dictate or scribble. The reply arrives as a notification.")
            }

            Section {
                Button(action: send) {
                    if isSending {
                        HStack(spacing: 6) {
                            ProgressView()
                            Text("Sending…")
                        }
                    } else {
                        Label("Send", systemImage: "paperplane.fill")
                    }
                }
                .disabled(trimmed.isEmpty || isSending)
            }

            if let receipt {
                Section {
                    ReceiptView(receipt: receipt)
                }
            }
        }
        .navigationTitle("Ask")
    }

    private func send() {
        guard !trimmed.isEmpty, !isSending else { return }
        isSending = true
        Task {
            let result = await store.quickAsk(trimmed)
            isSending = false
            receipt = result
            play(for: result.deliveryState)
            if result.deliveryState != .failed {
                prompt = ""
            }
        }
    }

    private func play(for state: QuickAskReceipt.DeliveryState) {
        #if canImport(WatchKit)
            switch state {
            case .sent: WKInterfaceDevice.current().play(.success)
            case .queued: WKInterfaceDevice.current().play(.notification)
            case .failed: WKInterfaceDevice.current().play(.failure)
            }
        #endif
    }
}

/// Delivery confirmation for a dictated prompt.
private struct ReceiptView: View {
    let receipt: QuickAskReceipt

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: icon)
                .foregroundStyle(tint)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.caption).bold()
                Text(receipt.text)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
    }

    private var title: String {
        switch receipt.deliveryState {
        case .sent: "Sent"
        case .queued: "Will send when reachable"
        case .failed: "Couldn't send"
        }
    }

    private var icon: String {
        switch receipt.deliveryState {
        case .sent: "checkmark.circle.fill"
        case .queued: "clock.arrow.circlepath"
        case .failed: "exclamationmark.triangle.fill"
        }
    }

    private var tint: Color {
        switch receipt.deliveryState {
        case .sent: .green
        case .queued: .orange
        case .failed: .red
        }
    }
}
