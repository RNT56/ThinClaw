import SwiftUI
import ThinClawCore
import ThinClawDesign

/// A single job's read-only detail: header, state transitions, and a live event
/// tail polled from `GET /api/jobs/{id}/events`.
///
/// The store starts the tail on ``open(id:)`` and stops it on ``close()``; this
/// view drives that lifecycle from `.task` / `.onDisappear`. Nothing here can
/// mutate the job — the phone holds `jobs:read` only, so the view is a live
/// read of a job the operator started elsewhere.
struct JobDetailScreen: View {
    @Bindable var store: JobsStore
    let jobID: String

    var body: some View {
        List {
            headerSection
            transitionsSection
            tailSection
        }
        .navigationTitle(store.detail?.title ?? "Job")
        .navigationBarTitleDisplayMode(.inline)
        .task(id: jobID) { await store.open(id: jobID) }
        .onDisappear { store.close() }
    }

    // MARK: - Header

    @ViewBuilder private var headerSection: some View {
        Section {
            if let detail = store.detail {
                VStack(alignment: .leading, spacing: ThinClawSpacing.sm) {
                    HStack(spacing: ThinClawSpacing.sm) {
                        Image(systemName: detail.phase.symbolName)
                            .foregroundStyle(detail.phase.tint)
                            .accessibilityHidden(true)
                        Text(detail.state)
                            .font(ThinClawTypography.cardTitle)
                        Spacer()
                        if store.isTailing {
                            liveBadge
                        }
                    }

                    if !detail.description.isEmpty {
                        Text(detail.description)
                            .font(ThinClawTypography.body)
                            .foregroundStyle(.secondary)
                    }

                    if let elapsed = detail.elapsedSeconds {
                        Text("Elapsed \(elapsed)s")
                            .font(ThinClawTypography.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                .accessibilityElement(children: .combine)
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
        } footer: {
            Label("Read-only — started from another surface.", systemImage: "eye")
                .font(ThinClawTypography.caption)
        }
    }

    private var liveBadge: some View {
        HStack(spacing: ThinClawSpacing.xs) {
            Circle().fill(.green).frame(width: 6, height: 6)
            Text("Live")
                .font(ThinClawTypography.caption)
                .foregroundStyle(.secondary)
        }
        .accessibilityLabel(Text("Live updating"))
    }

    // MARK: - Transitions

    @ViewBuilder private var transitionsSection: some View {
        if let detail = store.detail, !detail.transitions.isEmpty {
            Section("State transitions") {
                ForEach(detail.transitions) { transition in
                    HStack(spacing: ThinClawSpacing.sm) {
                        Text(transition.from)
                            .foregroundStyle(.secondary)
                        Image(systemName: "arrow.right")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                            .accessibilityHidden(true)
                        Text(transition.to)
                        Spacer()
                        if let timestamp = transition.timestamp {
                            Text(timestamp.formatted(date: .omitted, time: .shortened))
                                .font(ThinClawTypography.caption)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .font(ThinClawTypography.caption)
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(
                        Text("From \(transition.from) to \(transition.to)"))
                }
            }
        }
    }

    // MARK: - Event tail

    @ViewBuilder private var tailSection: some View {
        Section("Activity") {
            if let error = store.tailError, store.events.isEmpty {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
            } else if store.events.isEmpty {
                Text("Waiting for activity…")
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
            } else {
                ForEach(store.events) { event in
                    JobEventRow(event: event)
                }
            }
        }
    }
}

/// One event row in the live tail: a kind glyph, the extracted summary, and the
/// event time.
private struct JobEventRow: View {
    let event: JobEvent

    var body: some View {
        HStack(alignment: .top, spacing: ThinClawSpacing.sm) {
            Image(systemName: event.kind.symbolName)
                .font(.caption)
                .foregroundStyle(event.kind.tint)
                .frame(width: 16)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: 2) {
                Text(event.summary)
                    .font(ThinClawTypography.caption)
                if let createdAt = event.createdAt {
                    Text(createdAt.formatted(date: .omitted, time: .standard))
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text("\(event.kind.spokenLabel): \(event.summary)"))
    }
}

extension JobEvent.Kind {
    var symbolName: String {
        switch self {
        case .message: return "text.bubble"
        case .toolUse: return "wrench.and.screwdriver"
        case .toolResult: return "arrow.turn.down.right"
        case .result: return "flag.checkered"
        case .other: return "circle.dotted"
        }
    }

    var tint: Color {
        switch self {
        case .message: return .primary
        case .toolUse: return .blue
        case .toolResult: return .teal
        case .result: return .green
        case .other: return .secondary
        }
    }

    var spokenLabel: String {
        switch self {
        case .message: return "Message"
        case .toolUse: return "Tool call"
        case .toolResult: return "Tool result"
        case .result: return "Result"
        case .other: return "Event"
        }
    }
}
