import SwiftUI
import ThinClawCore
import ThinClawDesign

/// The streaming chat transcript for one thread: a flat list of
/// `TimelineItem`s, a slim offline/degraded banner with manual retry, and the
/// glass composer bar with a 429 cooldown.
public struct ChatScreen: View {
    @State private var store: ChatStore

    public init(store: ChatStore) {
        self._store = State(initialValue: store)
    }

    public var body: some View {
        VStack(spacing: 0) {
            if store.isOffline {
                offlineBanner
            }

            List {
                if store.hasMoreHistory {
                    historyLoader
                }
                ForEach(store.timeline) { item in
                    TimelineRow(item: item) {
                        Task { await store.retry(rowID: item.id) }
                    }
                    .listRowSeparator(.hidden)
                }
            }
            .listStyle(.plain)

            composer
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                StatusPill(store.connection)
            }
        }
        .task { await store.open() }
        .onDisappear { store.close() }
    }

    private var offlineBanner: some View {
        HStack(spacing: ThinClawSpacing.sm) {
            Image(systemName: "wifi.slash")
            Text("Offline — messages will send when reconnected")
                .font(ThinClawTypography.caption)
            Spacer()
            Button("Retry") {
                Task { await store.retryConnection() }
            }
            .font(ThinClawTypography.caption)
        }
        .padding(.horizontal, ThinClawSpacing.md)
        .padding(.vertical, ThinClawSpacing.sm)
        .frame(maxWidth: .infinity)
        .background(.orange.opacity(0.15))
    }

    private var historyLoader: some View {
        HStack {
            Spacer()
            ProgressView()
                .task { await store.loadOlderHistory() }
            Spacer()
        }
        .listRowSeparator(.hidden)
    }

    private var composer: some View {
        VStack(spacing: ThinClawSpacing.xs) {
            if store.cooldownRemaining > 0 {
                Text("Rate limited — try again in \(Int(store.cooldownRemaining.rounded(.up)))s")
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
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
                .disabled(store.isSendDisabled)
            }
        }
        .padding(.horizontal, ThinClawSpacing.md)
        .padding(.vertical, ThinClawSpacing.sm)
        .glassEffect(.regular, in: .rect(cornerRadius: ThinClawRadius.card))
        .padding(ThinClawSpacing.md)
    }
}

/// Renders one timeline item by kind.
struct TimelineRow: View {
    let item: TimelineItem
    /// Invoked when a failure row is tapped (retry). No-op for other kinds.
    var onRetry: () -> Void = {}

    /// Opens external URLs (the `auth_required` OAuth flow). The mobile client
    /// only *opens* the consent page — per D-T4 it never captures the returned
    /// token, so completing the flow hands off to the desktop.
    @Environment(\.openURL) private var openURL

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
        case .authPrompt(let prompt):
            AuthPromptCard(
                extensionName: prompt.extensionName,
                instructions: prompt.instructions,
                hasAuthURL: prompt.authURL != nil,
                onOpenAuth: {
                    if let url = prompt.authURL { openURL(url) }
                })
        case .credentialPrompt(let prompt):
            CredentialPromptCard(
                provider: prompt.provider,
                secretName: prompt.secretName,
                reason: prompt.reason)
        case .failure(let message):
            Button(action: onRetry) {
                Label(message, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.red)
            }
            .buttonStyle(.plain)
        }
    }
}
