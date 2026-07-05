import SwiftUI
import ThinClawCore
import ThinClawDesign

/// Read-only glance at background jobs (`jobs:read` scope).
@MainActor
@Observable
public final class JobsStore {
    public struct JobRow: Identifiable, Hashable, Sendable {
        public let id: String
        public var title: String
        public var state: String

        public init(id: String, title: String, state: String) {
            self.id = id
            self.title = title
            self.state = state
        }
    }

    public private(set) var jobs: [JobRow] = []

    public init() {}

    /// M5: GET /api/jobs + per-job event tail.
    public func refresh() async {}
}

public struct JobsScreen: View {
    @State private var store: JobsStore

    public init(store: JobsStore = JobsStore()) {
        self._store = State(initialValue: store)
    }

    public var body: some View {
        List(store.jobs) { job in
            HStack {
                Text(job.title)
                    .font(ThinClawTypography.body)
                Spacer()
                Text(job.state)
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .overlay {
            if store.jobs.isEmpty {
                ContentUnavailableView(
                    "No background jobs",
                    systemImage: "clock.badge.checkmark"
                )
            }
        }
        .navigationTitle("Jobs")
        .refreshable { await store.refresh() }
    }
}
