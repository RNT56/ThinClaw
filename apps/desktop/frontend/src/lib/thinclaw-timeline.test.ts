import { describe, expect, it } from 'vitest';

import type { ThinClawMessage } from './thinclaw';
import { buildThinClawTimeline } from './thinclaw-timeline';

function message(index: number, role: ThinClawMessage['role'] = 'assistant'): ThinClawMessage {
    return {
        id: `message-${index}`,
        role,
        ts_ms: index,
        text: `message ${index}`,
        source: 'thinclaw',
    };
}

describe('buildThinClawTimeline', () => {
    it('prepares a large history in one bounded pass for the virtualized view', () => {
        const messages = Array.from({ length: 10_000 }, (_, index) => message(index));
        const startedAt = performance.now();
        const timeline = buildThinClawTimeline(messages, false);

        expect(timeline).toHaveLength(10_000);
        expect(performance.now() - startedAt).toBeLessThan(1_000);
    });

    it('keeps internal and tool traffic out of core chat mode', () => {
        const messages = [
            message(1, 'user'),
            { ...message(2, 'system'), text: '[Tool Call: shell]' },
            { ...message(3), text: '🧠 private reasoning' },
            message(4),
        ];

        expect(buildThinClawTimeline(messages, true).map((item) => item.data.items[0].id))
            .toEqual(['message-1', 'message-4']);
    });
});
