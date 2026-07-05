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
        List(store.threads) { thread in
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
        }
        .navigationTitle("Sessions")
        .refreshable { await store.refresh() }
        .task { await store.load() }
    }
}
