import SwiftUI
import ThinClawSnapshotKit

/// Watch root: glanceable status, pending approvals (low-risk actionable),
/// and a dictated quick prompt. Data arrives relay-first over
/// WatchConnectivity (M4); the watch holds its own reduced-scope token.
struct WatchRootView: View {
    var body: some View {
        NavigationStack {
            List {
                Section {
                    Label("Not paired yet", systemImage: "iphone.and.arrow.forward")
                        .font(.caption)
                } footer: {
                    Text("Pair the iPhone app first; the watch provisions automatically.")
                }

                Section("Actions") {
                    NavigationLink {
                        ApprovalsListView()
                    } label: {
                        Label("Approvals", systemImage: "checkmark.shield")
                    }
                    Button {
                        // M4: dictated quick prompt via relay.
                    } label: {
                        Label("Ask ThinClaw", systemImage: "mic")
                    }
                }
            }
            .navigationTitle("ThinClaw")
        }
    }
}

/// Low-risk approvals only; high-risk requests say "approve on iPhone"
/// (docs/MOBILE_SECURITY.md, D-K3/D-K4).
struct ApprovalsListView: View {
    var body: some View {
        ContentUnavailableView(
            "No pending approvals",
            systemImage: "checkmark.shield"
        )
    }
}
