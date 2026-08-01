/**
 * Transport success is not the same as an applied action. This adapter keeps
 * page copy aligned with the structured result returned by a ThinClaw command.
 */
export type AgentActionOutcomeState = 'applied' | 'persisted' | 'prepared' | 'restart-required' | 'rejected' | 'unchanged' | 'unknown';

export interface AgentActionOutcome {
    state: AgentActionOutcomeState;
    message: string;
}

export function normalizeAgentActionOutcome(value: unknown, fallback: string): AgentActionOutcome {
    if (!value || typeof value !== 'object') return { state: 'unknown', message: fallback };
    const record = value as Record<string, unknown>;
    const note = [record.note, record.message, record.reason, record.status]
        .find((candidate): candidate is string => typeof candidate === 'string' && candidate.trim().length > 0) ?? fallback;

    if (record.restart_required === true || record.restartRequired === true) {
        return { state: 'restart-required', message: note };
    }
    if (record.persisted === true && (record.forwarded === false || record.live_applied === false || record.liveApplied === false || record.ok === false)) {
        return { state: 'persisted', message: note };
    }
    if (record.ok === false || record.success === false || record.rejected === true) {
        return { state: 'rejected', message: note };
    }
    if (record.prepared === true) return { state: 'prepared', message: note };
    if (record.unchanged === true) return { state: 'unchanged', message: note };
    if (record.ok === true || record.success === true || typeof record.status === 'string') {
        return { state: 'applied', message: note };
    }
    return { state: 'unknown', message: note };
}
