import SwiftUI
import ThinClawCore
import ThinClawDesign

/// Thread list: the assistant thread plus side conversations, cached locally
/// for an instant offline first paint, refreshed from the gateway.
public struct SessionsScreen: View {
    @State private var store: SessionsStore
    private let onSelect: (ThreadID) -> Void

    public init(store: SessionsStore, onSelect: @escaping (ThreadID) -> Void = { _ in }) {
        self._store = State(initialValue: store)
        self.onSelect = onSelect
    }

    public var body: some View {
        List {
            if let error = store.errorMessage {
                Section {
                    Label(error, systemImage: "wifi.exclamationmark")
                        .foregroundStyle(.orange)
                }
            }
            if store.threads.isEmpty, store.hasRefreshed {
                ContentUnavailableView(
                    "No sessions",
                    systemImage: "list.bullet.rectangle",
                    description: Text("New conversations appear here.")
                )
                .listRowBackground(Color.clear)
            } else {
                ForEach(store.threads) { thread in
                    Button {
                        onSelect(thread.id)
                    } label: {
                        VStack(alignment: .leading, spacing: ThinClawSpacing.xs) {
                            Text(thread.title)
                                .font(ThinClawTypography.cardTitle)
                            if let preview = thread.lastMessagePreview {
                                Text(preview)
                                    .font(ThinClawTypography.caption)
                                    .foregroundStyle(.secondary)
                                    .lineLimit(2)
                            }
                        }
                    }
                    .accessibilityIdentifier(SessionsPresentation.accessibilityIdentifier(for: thread.id))
                }
            }
            if store.isShowingCachedData {
                Text("Showing saved sessions. Pull to refresh when you're online.")
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .navigationTitle("Sessions")
        .refreshable { await store.refresh() }
        .task { await store.load() }
        .overlay {
            if store.isLoading && store.threads.isEmpty {
                ProgressView("Loading sessions…")
            }
        }
    }
}

enum SessionsPresentation {
    static func accessibilityIdentifier(for threadID: ThreadID) -> String {
        "session.\(threadID.rawValue)"
    }
}
