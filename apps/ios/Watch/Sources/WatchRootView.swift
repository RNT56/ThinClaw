import SwiftUI
import ThinClawSnapshotKit
import ThinClawWatchBridge

/// Watch root: glanceable agent status (phase + pending count + transport
/// route badge), and a NavigationStack into Approvals and Ask.
///
/// Data arrives relay-first over WatchConnectivity (docs/MOBILE_SECURITY.md
/// D-K4); the watch holds its own reduced-scope token and only ever approves
/// low-risk requests on the wrist (D-K3).
struct WatchRootView: View {
    @State var store: WatchStore

    var body: some View {
        NavigationStack {
            List {
                statusSection

                Section("Actions") {
                    NavigationLink {
                        ApprovalsListView(store: store)
                    } label: {
                        Label {
                            HStack {
                                Text("Approvals")
                                if store.pendingCount > 0 {
                                    Spacer()
                                    Text("\(store.pendingCount)")
                                        .font(.caption2)
                                        .foregroundStyle(.orange)
                                }
                            }
                        } icon: {
                            Image(systemName: "checkmark.shield")
                        }
                    }

                    NavigationLink {
                        AskView(store: store)
                    } label: {
                        Label("Ask ThinClaw", systemImage: "mic")
                    }
                }
            }
            .navigationTitle("ThinClaw")
            .task { await store.refresh() }
            .refreshable { await store.refresh() }
        }
    }

    // MARK: - Glanceable status

    @ViewBuilder
    private var statusSection: some View {
        Section {
            if let status = store.status {
                HStack(spacing: ThinClawWatchStyle.rowSpacing) {
                    Image(systemName: WatchPhasePresentation.icon(status.phase))
                        .foregroundStyle(WatchPhasePresentation.tint(status.phase))
                    VStack(alignment: .leading, spacing: 2) {
                        Text(WatchPhasePresentation.label(status.phase))
                            .font(.headline)
                            .lineLimit(1)
                        if let title = status.activeThreadTitle {
                            Text(title)
                                .font(.caption2)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                    }
                    Spacer()
                }
                RouteBadge(route: store.route)
                if SnapshotStaleness.isStale(status) {
                    Text(
                        "Stale as of \(status.generatedAt.formatted(.relative(presentation: .named)))"
                    )
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                }
            } else {
                VStack(alignment: .leading, spacing: 4) {
                    Label("Not paired yet", systemImage: "iphone.and.arrow.forward")
                        .font(.caption)
                    RouteBadge(route: store.route)
                }
            }
        } footer: {
            if !store.hasSnapshot {
                Text("Pair the iPhone app first; the watch provisions automatically.")
            }
        }
    }
}

/// Honest transport badge: relay through the phone, direct to a reachable
/// gateway, or queued until reachable (docs/MOBILE_APP.md, watch section).
struct RouteBadge: View {
    let route: WatchRoute

    var body: some View {
        Label(title, systemImage: icon)
            .font(.caption2)
            .foregroundStyle(tint)
            .accessibilityLabel("Connection route: \(title)")
    }

    private var title: String {
        switch route {
        case .relay: "via iPhone"
        case .direct: "direct"
        case .queued: "pending sync"
        }
    }

    private var icon: String {
        switch route {
        case .relay: "iphone.gen3"
        case .direct: "wifi"
        case .queued: "clock.arrow.circlepath"
        }
    }

    private var tint: Color {
        switch route {
        case .relay: .green
        case .direct: .blue
        case .queued: .orange
        }
    }
}

/// Phase → glanceable label / icon / tint, watch-sized. Mirrors the iOS widget
/// vocabulary so the two surfaces read the same.
enum WatchPhasePresentation {
    static func label(_ phase: AgentStatusSnapshot.Phase) -> String {
        switch phase {
        case .idle: "Idle"
        case .thinking: "Thinking…"
        case .streaming: "Responding…"
        case .runningTool: "Running a tool"
        case .waitingForApproval: "Needs approval"
        case .error: "Error"
        }
    }

    static func icon(_ phase: AgentStatusSnapshot.Phase) -> String {
        switch phase {
        case .idle: "moon.zzz"
        case .thinking: "brain"
        case .streaming: "text.bubble"
        case .runningTool: "gearshape.2"
        case .waitingForApproval: "checkmark.shield"
        case .error: "exclamationmark.triangle"
        }
    }

    static func tint(_ phase: AgentStatusSnapshot.Phase) -> Color {
        switch phase {
        case .idle: .secondary
        case .thinking, .streaming: .blue
        case .runningTool: .indigo
        case .waitingForApproval: .orange
        case .error: .red
        }
    }
}

/// Small layout constants tuned for the wrist (screen-edge budget is tighter
/// than iOS). ThinClawDesign's spacing tokens are iOS-glass sized; the watch
/// uses its own compact scale.
enum ThinClawWatchStyle {
    static let rowSpacing: CGFloat = 8
}

/// Watch-local staleness check. The equivalent `isStale` lives in
/// ThinClawWidgetKitShared, which is iOS/WidgetKit-scoped and not a watch
/// dependency, so we inline the same 30-minute threshold here rather than pull
/// that package (and WidgetKit) into the watch app.
enum SnapshotStaleness {
    static func isStale(
        _ snapshot: AgentStatusSnapshot,
        asOf now: Date = .now,
        maxAge: TimeInterval = 30 * 60
    ) -> Bool {
        now.timeIntervalSince(snapshot.generatedAt) > maxAge
    }
}
