import type { UiEvent } from './bindings';
import { PERFORMANCE_BUDGETS } from './performance-budgets';

export const STREAM_EVENT_FLUSH_INTERVAL_MS = PERFORMANCE_BUDGETS.streamEventFlushMs;
export const MAX_COALESCED_DELTA_CHARS = 64 * 1024;

type TimerHandle = ReturnType<typeof setTimeout>;
type Scheduler = (callback: () => void, delayMs: number) => TimerHandle;
type Canceller = (handle: TimerHandle) => void;

/**
 * Coalesce adjacent token deltas for one message into at most one frontend
 * delivery per animation-frame budget. All other event variants remain ordered
 * and are dispatched immediately after pending deltas are flushed.
 */
export class ThinClawEventBuffer {
    private pendingDeltas: UiEvent[] = [];
    private timer: TimerHandle | null = null;

    constructor(
        private readonly dispatch: (event: UiEvent) => void,
        private readonly schedule: Scheduler = setTimeout,
        private readonly cancel: Canceller = clearTimeout,
    ) { }

    push(event: UiEvent): void {
        if (event.kind !== 'AssistantDelta') {
            this.flush();
            this.dispatch(event);
            return;
        }

        const previous = this.pendingDeltas[this.pendingDeltas.length - 1];
        if (
            previous?.kind === 'AssistantDelta'
            && previous.session_key === event.session_key
            && previous.message_id === event.message_id
            && previous.run_id === event.run_id
            && previous.delta.length + event.delta.length <= MAX_COALESCED_DELTA_CHARS
        ) {
            previous.delta += event.delta;
        } else {
            this.pendingDeltas.push({ ...event });
        }

        if (this.timer === null) {
            this.timer = this.schedule(() => this.flush(), STREAM_EVENT_FLUSH_INTERVAL_MS);
        }
    }

    flush(): void {
        if (this.timer !== null) {
            this.cancel(this.timer);
            this.timer = null;
        }
        const pending = this.pendingDeltas;
        this.pendingDeltas = [];
        for (const event of pending) this.dispatch(event);
    }

    dispose(): void {
        this.flush();
    }
}
