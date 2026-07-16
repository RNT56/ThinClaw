import { describe, expect, it, vi } from 'vitest';

import type { UiEvent } from './bindings';
import {
    MAX_COALESCED_DELTA_CHARS,
    STREAM_EVENT_FLUSH_INTERVAL_MS,
    ThinClawEventBuffer,
} from './thinclaw-event-buffer';

function delta(text: string, messageId = 'message-1'): UiEvent {
    return {
        kind: 'AssistantDelta',
        session_key: 'agent:main',
        message_id: messageId,
        run_id: 'run-1',
        delta: text,
    };
}

describe('ThinClawEventBuffer', () => {
    it('coalesces a token burst into one delivery per frame budget', () => {
        vi.useFakeTimers();
        const delivered: UiEvent[] = [];
        const buffer = new ThinClawEventBuffer((event) => delivered.push(event));

        for (let index = 0; index < 1_000; index += 1) buffer.push(delta('x'));
        expect(delivered).toHaveLength(0);

        vi.advanceTimersByTime(STREAM_EVENT_FLUSH_INTERVAL_MS);
        expect(delivered).toEqual([delta('x'.repeat(1_000))]);
        vi.useRealTimers();
    });

    it('flushes deltas before a non-stream event without reordering', () => {
        vi.useFakeTimers();
        const delivered: UiEvent[] = [];
        const buffer = new ThinClawEventBuffer((event) => delivered.push(event));
        const final: UiEvent = {
            kind: 'AssistantFinal',
            session_key: 'agent:main',
            message_id: 'message-1',
            run_id: 'run-1',
            text: 'ab',
            usage: null,
        };

        buffer.push(delta('a'));
        buffer.push(delta('b'));
        buffer.push(final);

        expect(delivered).toEqual([delta('ab'), final]);
        vi.useRealTimers();
    });

    it('bounds a single coalesced payload', () => {
        vi.useFakeTimers();
        const delivered: UiEvent[] = [];
        const buffer = new ThinClawEventBuffer((event) => delivered.push(event));
        const chunk = 'x'.repeat(MAX_COALESCED_DELTA_CHARS);

        buffer.push(delta(chunk));
        buffer.push(delta('y'));
        vi.advanceTimersByTime(STREAM_EVENT_FLUSH_INTERVAL_MS);

        expect(delivered).toEqual([delta(chunk), delta('y')]);
        vi.useRealTimers();
    });
});
