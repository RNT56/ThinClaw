import SwiftUI
import ThinClawCore
import ThinClawDesign

/// The streaming chat transcript for one thread: a flat list of
/// `TimelineItem`s, a slim offline/degraded banner with manual retry, and the
/// glass composer bar with a 429 cooldown.
public struct ChatScreen: View {
    @State private var store: ChatStore
    private let approvalsStore: ApprovalsStore?

    public init(store: ChatStore, approvalsStore: ApprovalsStore? = nil) {
        self._store = State(initialValue: store)
        self.approvalsStore = approvalsStore
    }

    public var body: some View {
        VStack(spacing: 0) {
            if store.isOffline {
                offlineBanner
            }

            ScrollViewReader { proxy in
                List {
                    if store.hasMoreHistory {
                        historyLoader
                    }
                    ForEach(store.timeline) { item in
                        TimelineRow(
                            item: item,
                            onRetry: { Task { await store.retry(rowID: item.id) } },
                            onDelete: { Task { await store.delete(rowID: item.id) } },
                            onApprove: { requestID in
                                Task { await approvalsStore?.approve(requestID) }
                            },
                            onDeny: { requestID in
                                Task { await approvalsStore?.deny(requestID) }
                            }
                        )
                        .id(item.id)
                        .listRowSeparator(.hidden)
                    }
                }
                .listStyle(.plain)
                .scrollDismissesKeyboard(.interactively)
                .onChange(of: store.timeline.last?.id) { _, latest in
                    guard let latest else { return }
                    withAnimation(.easeOut(duration: 0.2)) {
                        proxy.scrollTo(latest, anchor: .bottom)
                    }
                }
                .task {
                    guard let latest = store.timeline.last?.id else { return }
                    proxy.scrollTo(latest, anchor: .bottom)
                }
            }

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
                .accessibilityHidden(true)
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
        // Group the icon + copy as one label; the Retry button stays a
        // separately focusable action.
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text("Offline. Messages will send when reconnected."))
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
                    .accessibilityIdentifier("chat.composer")
                Button {
                    Task { await store.send() }
                } label: {
                    Image(systemName: "arrow.up.circle.fill")
                        .font(.title2)
                }
                .disabled(store.isSendDisabled)
                .frame(minWidth: 44, minHeight: 44)
                .accessibilityLabel("Send message")
                .accessibilityIdentifier("chat.send")
            }
        }
        .padding(.horizontal, ThinClawSpacing.md)
        .padding(.vertical, ThinClawSpacing.sm)
        .thinClawSurface()
        .padding(ThinClawSpacing.md)
    }
}

/// Renders one timeline item by kind.
struct TimelineRow: View {
    let item: TimelineItem
    /// Invoked when a failure row is tapped (retry). No-op for other kinds.
    var onRetry: () -> Void = {}
    var onDelete: () -> Void = {}
    var onApprove: (String) -> Void = { _ in }
    var onDeny: (String) -> Void = { _ in }

    /// Opens external URLs (the `auth_required` OAuth flow). The mobile client
    /// only *opens* the consent page — per D-T4 it never captures the returned
    /// token, so completing the flow hands off to the desktop.
    @Environment(\.openURL) private var openURL

    var body: some View {
        rowContent
            // VoiceOver: one spoken element per row, worded by the pure
            // `TimelineAccessibility` descriptor in ThinClawCore. Streaming
            // replies keep a stable label and announce growth via the value.
            .accessibilityElement(children: .combine)
            .modifier(TimelineAccessibilityModifier(descriptor: item.accessibility))
    }

    @ViewBuilder
    private var rowContent: some View {
        switch item.kind {
        case .userMessage(let text):
            VStack(alignment: .trailing, spacing: ThinClawSpacing.xs) {
                Text(text)
                    .font(ThinClawTypography.body)
                if let deliveryState = item.deliveryState {
                    Label(deliveryState.label, systemImage: deliveryState.symbolName)
                        .font(.caption2)
                        .foregroundStyle(
                            deliveryState == .failed
                                ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                    if deliveryState == .failed {
                        HStack(spacing: ThinClawSpacing.sm) {
                            Button("Retry", action: onRetry)
                            Button("Delete", role: .destructive, action: onDelete)
                        }
                        .buttonStyle(.borderless)
                        .frame(minHeight: 44)
                    }
                }
            }
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
            ApprovalCard(
                toolName: request.toolName,
                requestDescription: request.description,
                parameters: request.parameters,
                risk: request.risk == .low ? .low : .high,
                onApprove: { onApprove(request.requestID) },
                onDeny: { onDeny(request.requestID) })
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

extension TimelineItem.DeliveryState {
    fileprivate var label: String {
        switch self {
        case .sending: "Sending"
        case .queued: "Queued"
        case .failed: "Not sent"
        }
    }

    fileprivate var symbolName: String {
        switch self {
        case .sending: "arrow.up.circle"
        case .queued: "clock"
        case .failed: "exclamationmark.triangle"
        }
    }
}

/// Applies a pure ``TimelineAccessibility`` descriptor to a row: label, an
/// optional value (streaming prose, announced politely on change), and an
/// optional action hint.
private struct TimelineAccessibilityModifier: ViewModifier {
    let descriptor: TimelineAccessibility

    func body(content: Content) -> some View {
        content
            .accessibilityLabel(Text(descriptor.label))
            .accessibilityValue(Text(descriptor.value ?? ""))
            .accessibilityHint(Text(descriptor.hint ?? ""))
    }
}
