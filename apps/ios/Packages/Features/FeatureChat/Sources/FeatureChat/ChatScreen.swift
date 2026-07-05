import SwiftUI
import ThinClawCore
import ThinClawDesign

/// The streaming chat transcript for one thread: a flat list of
/// `TimelineItem`s with the glass composer bar.
@MainActor
@Observable
public final class ChatStore {
    public private(set) var timeline: [TimelineItem] = []
    public private(set) var connection: StatusPill.Status = .offline
    public var draft: String = ""

    public init() {}

    /// M1: fold `AgentEvent`s (routed per-thread by the EventRouter) into
    /// timeline items via `StreamChunkCoalescer`.
    public func apply(_ event: AgentEvent) {}

    /// M1: POST /api/chat/send, or enqueue to the outbox while offline.
    public func send() async {}
}

public struct ChatScreen: View {
    @State private var store: ChatStore

    public init(store: ChatStore = ChatStore()) {
        self._store = State(initialValue: store)
    }

    public var body: some View {
        VStack(spacing: 0) {
            List(store.timeline) { item in
                TimelineRow(item: item)
                    .listRowSeparator(.hidden)
            }
            .listStyle(.plain)

            composer
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                StatusPill(store.connection)
            }
        }
    }

    private var composer: some View {
        HStack(spacing: ThinClawSpacing.sm) {
            TextField("Message ThinClaw…", text: $store.draft, axis: .vertical)
                .lineLimit(1...5)
                .textFieldStyle(.plain)
                .padding(ThinClawSpacing.md)
            Button {
                Task { await store.send() }
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.title2)
            }
            .disabled(store.draft.isEmpty)
        }
        .padding(.horizontal, ThinClawSpacing.md)
        .padding(.vertical, ThinClawSpacing.sm)
        .glassEffect(.regular, in: .rect(cornerRadius: ThinClawRadius.card))
        .padding(ThinClawSpacing.md)
    }
}

/// Renders one timeline item by kind. Grows with M1.
struct TimelineRow: View {
    let item: TimelineItem

    var body: some View {
        switch item.kind {
        case .userMessage(let text):
            Text(text)
                .font(ThinClawTypography.body)
                .frame(maxWidth: .infinity, alignment: .trailing)
        case .agentMessage(let text):
            StreamingText(text, isStreaming: false)
        case .streamingAgentMessage(let text):
            StreamingText(text, isStreaming: true)
        case .statusNote(let text):
            Text(text)
                .font(ThinClawTypography.caption)
                .foregroundStyle(.secondary)
        case .toolCall(let name, let status):
            Label(name, systemImage: status == .running ? "gearshape.2" : "checkmark.circle")
                .font(ThinClawTypography.caption)
        case .approval(let request):
            Text("Approval requested: \(request.toolName)")
                .font(ThinClawTypography.caption)
        case .failure(let message):
            Label(message, systemImage: "exclamationmark.triangle")
                .foregroundStyle(.red)
        }
    }
}
