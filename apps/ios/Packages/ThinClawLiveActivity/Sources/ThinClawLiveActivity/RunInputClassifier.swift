import Foundation
import ThinClawCore

/// Pure mapping from a raw ``AgentEvent`` to a ``RunInput`` for a specific
/// *active* thread. Kept separate from ``RunTracker`` (and Foundation-only) so
/// the "which events drive the Live Activity" policy is unit-tested without
/// ActivityKit.
///
/// Only events belonging to `activeThread` produce inputs; events for other
/// threads (or thread-less events like `heartbeat`) return `nil`, because the
/// manager owns at most one activity for the active thread.
public enum RunInputClassifier {
    /// Map `event` to a ``RunInput`` for `activeThread`, or `nil` if the event
    /// does not drive the activity.
    ///
    /// - Parameter threadTitle: The title to stamp on a `.runStarted` when this
    ///   event is the first signal for the thread. Only used for the start
    ///   inputs; ignored otherwise.
    public static func input(
        from event: AgentEvent,
        activeThread: ThreadID,
        threadTitle: String
    ) -> RunInput? {
        // Route only events for the active thread. Thread-less events never
        // drive the activity.
        guard let eventThread = event.threadID, eventThread == activeThread else {
            return nil
        }

        switch event {
        case .thinking:
            return .thinking(threadID: activeThread)

        case .toolStarted(let name, _):
            return .toolStarted(threadID: activeThread, toolName: name)

        case .toolCompleted:
            // A completed tool returns the run to "thinking" until the next
            // signal; keeps the activity from appearing stuck on a finished
            // tool. (Terminal-tool name is dropped — content-free.)
            return .thinking(threadID: activeThread)

        case .approvalNeeded(let request):
            return .awaitingApproval(threadID: activeThread, requestID: request.requestID)

        case .response:
            return .completed(threadID: activeThread)

        case .error:
            return .failed(threadID: activeThread)

        // No activity signal: streaming text (content — never in the activity),
        // usage accounting, auth/credential prompts, heartbeat, unknowns.
        case .streamChunk, .usageUpdate, .authRequired, .credentialPrompt, .heartbeat,
            .unknown:
            return nil
        }
    }

    /// Whether `event` for `activeThread` should *begin* tracking a run when
    /// none is active yet. The first `thinking`/`tool_started`/`approval_needed`
    /// for the thread starts the activity; a bare `response`/`error` for an
    /// untracked thread does not (there is nothing to end).
    public static func isRunStart(_ event: AgentEvent, activeThread: ThreadID) -> Bool {
        guard let eventThread = event.threadID, eventThread == activeThread else {
            return false
        }
        switch event {
        case .thinking, .toolStarted, .approvalNeeded:
            return true
        default:
            return false
        }
    }
}
