#if os(iOS) && canImport(ActivityKit)
    import ActivityKit
    import Foundation

    /// Live Activity attributes for an agent run.
    ///
    /// ContentState is deliberately content-free: a phase enum, optional tool
    /// name, and progress — never prompt text or tool arguments, because
    /// ActivityKit updates transit APNs and render on the lock screen
    /// (docs/MOBILE_SECURITY.md, D-N2). `revision` is monotonically
    /// increasing so a late push never regresses UI driven by local SSE
    /// updates while the app is foregrounded.
    ///
    /// Lives in SnapshotKit (behind canImport) so the app target (which
    /// requests activities and registers push tokens) and the widget
    /// extension (which renders them) share one definition.
    public struct AgentRunAttributes: ActivityAttributes {
        public struct ContentState: Codable, Hashable {
            /// The backend's `push_policy` emits `phase` as a bare string
            /// (`"thinking"`, `"runningTool"`, `"awaitingApproval"`, `"done"`,
            /// `"failed"`), so this must be a `String`-raw-valued enum: the
            /// implicit raw values match the wire exactly. A raw-value-less
            /// enum would (de)serialize as `{"runningTool":{}}`, which does not
            /// match what the gateway sends (see
            /// `crates/thinclaw-gateway/src/web/devices/push_policy.rs`).
            public enum RunPhase: String, Codable, Hashable {
                case thinking
                case runningTool
                case awaitingApproval
                case done
                case failed
            }

            public var phase: RunPhase
            public var toolName: String?
            /// 0–100 when the run reports progress.
            public var progress: Int?
            public var pendingApprovalID: String?
            public var revision: UInt64

            public init(
                phase: RunPhase,
                toolName: String? = nil,
                progress: Int? = nil,
                pendingApprovalID: String? = nil,
                revision: UInt64
            ) {
                self.phase = phase
                self.toolName = toolName
                self.progress = progress
                self.pendingApprovalID = pendingApprovalID
                self.revision = revision
            }
        }

        public var threadID: String
        public var threadTitle: String

        public init(threadID: String, threadTitle: String) {
            self.threadID = threadID
            self.threadTitle = threadTitle
        }
    }
#endif
