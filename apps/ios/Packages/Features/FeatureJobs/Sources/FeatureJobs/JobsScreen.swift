import SwiftUI
import ThinClawCore
import ThinClawDesign

/// Read-only glance at background jobs (`jobs:read` scope).
///
/// The store (`ThinClawCore.JobsStore`) is UI-free and macOS-tested; this view
/// is the thin iOS shell. The phone can only *observe* jobs — there is no
/// cancel/restart/prompt affordance anywhere on this surface because those
/// endpoints are `POST` and not device-scoped (`docs/MOBILE_SECURITY.md`). The
/// list makes the read-only nature explicit with a footer note, and the detail
/// view tails the job's event log by polling `GET /api/jobs/{id}/events`.
public struct JobsScreen: View {
    @State private var store: JobsStore?
    @Binding private var selectedJobID: String?

    private let makeStore: @MainActor () -> JobsStore?

    /// - Parameter store: A factory that builds the wired ``JobsStore`` (nil
    ///   when unpaired). A factory rather than a value so the view owns the
    ///   store lifecycle and the app can hand it the live gateway adapter.
    public init(
        store: @escaping @MainActor () -> JobsStore? = { nil },
        selectedJobID: Binding<String?> = .constant(nil)
    ) {
        self.makeStore = store
        self._selectedJobID = selectedJobID
    }

    public var body: some View {
        Group {
            if let store {
                JobsList(store: store, selectedJobID: $selectedJobID)
            } else {
                ContentUnavailableView(
                    "Jobs unavailable",
                    systemImage: "clock.badge.xmark",
                    description: Text("Pair this device to view background jobs.")
                )
            }
        }
        .navigationTitle("Jobs")
        .task {
            if store == nil { store = makeStore() }
            await store?.refresh()
        }
    }
}

/// The jobs list: a summary header, the newest-first job rows, an empty state,
/// and a read-only footer. Tapping a row pushes the detail + live tail.
private struct JobsList: View {
    @Bindable var store: JobsStore
    @Binding var selectedJobID: String?

    var body: some View {
        List {
            if let error = store.listError {
                Section {
                    VStack(alignment: .leading, spacing: ThinClawSpacing.sm) {
                        Label(error, systemImage: "wifi.exclamationmark")
                            .foregroundStyle(.orange)
                        Button("Try again") { Task { await store.refresh() } }
                    }
                }
            }
            if let summary = store.summary, summary.total > 0 {
                Section {
                    JobsSummaryRow(summary: summary)
                }
            }

            if store.jobs.isEmpty {
                Section {
                    ContentUnavailableView(
                        "No background jobs",
                        systemImage: "clock.badge.checkmark",
                        description: Text("Jobs your agent runs in the background appear here.")
                    )
                    .listRowBackground(Color.clear)
                }
            } else {
                Section {
                    ForEach(store.jobs) { job in
                        Button {
                            selectedJobID = job.id
                        } label: {
                            JobRow(job: job)
                        }
                        .buttonStyle(.plain)
                    }
                } footer: {
                    Label(
                        "View only. Jobs can't be cancelled or restarted from this device.",
                        systemImage: "eye"
                    )
                    .font(ThinClawTypography.caption)
                }
            }
        }
        .refreshable { await store.refresh() }
        .overlay {
            if store.isLoadingList && store.jobs.isEmpty {
                ProgressView()
            }
        }
        .navigationDestination(item: $selectedJobID) { id in
            JobDetailScreen(store: store, jobID: id)
        }
    }
}

/// One job row: a phase glyph, the title, and the raw state + relative start.
private struct JobRow: View {
    let job: Job

    var body: some View {
        HStack(spacing: ThinClawSpacing.md) {
            Image(systemName: job.phase.symbolName)
                .foregroundStyle(job.phase.tint)
                .font(.title3)
                .accessibilityHidden(true)

            VStack(alignment: .leading, spacing: ThinClawSpacing.xs) {
                Text(job.title)
                    .font(ThinClawTypography.body)
                    .lineLimit(1)
                Text(job.subtitle)
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()
            Image(systemName: "chevron.right")
                .font(.caption)
                .foregroundStyle(.tertiary)
                .accessibilityHidden(true)
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(job.accessibilityLabel))
    }
}

/// The summary chips: active / completed / failed counts.
private struct JobsSummaryRow: View {
    let summary: JobsSummary

    var body: some View {
        HStack(spacing: ThinClawSpacing.lg) {
            SummaryChip(count: summary.active, label: "Active", tint: .blue)
            SummaryChip(count: summary.completed, label: "Done", tint: .green)
            SummaryChip(count: summary.failed, label: "Failed", tint: .red)
        }
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            Text(
                "\(summary.active) active, \(summary.completed) done, \(summary.failed) failed"
            ))
    }

    private struct SummaryChip: View {
        let count: Int
        let label: String
        let tint: Color

        var body: some View {
            VStack(spacing: ThinClawSpacing.xs) {
                Text("\(count)")
                    .font(.title2.weight(.semibold))
                    .foregroundStyle(tint)
                Text(label)
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

// MARK: - Phase presentation

extension JobPhase {
    var symbolName: String {
        switch self {
        case .pending: return "clock"
        case .running: return "arrow.triangle.2.circlepath"
        case .succeeded: return "checkmark.circle.fill"
        case .failed: return "xmark.octagon.fill"
        case .cancelled: return "slash.circle"
        case .stuck: return "exclamationmark.triangle.fill"
        case .unknown: return "questionmark.circle"
        }
    }

    var tint: Color {
        switch self {
        case .pending: return .yellow
        case .running: return .blue
        case .succeeded: return .green
        case .failed: return .red
        case .cancelled: return .secondary
        case .stuck: return .orange
        case .unknown: return .secondary
        }
    }
}

extension Job {
    /// The secondary line: raw state plus a relative start time when known.
    fileprivate var subtitle: String {
        if let startedAt {
            return "\(state) · \(startedAt.formatted(.relative(presentation: .named)))"
        }
        return state
    }

    fileprivate var accessibilityLabel: String {
        "\(title). \(subtitle)"
    }
}
