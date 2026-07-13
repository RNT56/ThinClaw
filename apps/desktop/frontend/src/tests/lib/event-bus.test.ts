import { describe, expect, it, vi } from 'vitest';

const eventMock = vi.hoisted(() => ({
    callback: null as ((event: { payload: unknown }) => void) | null,
    listen: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
    listen: eventMock.listen,
}));

import { subscribeThinClawEvents } from '../../hooks/use-thinclaw-stream';
import type { UiEvent } from '../../lib/bindings';

describe('ThinClaw event bus', () => {
    it('starts one native listener and fans typed events out to active subscribers', async () => {
        eventMock.listen.mockImplementationOnce(async (_name, callback) => {
            eventMock.callback = callback;
            return vi.fn();
        });
        const first = vi.fn();
        const second = vi.fn();
        const unsubscribeFirst = subscribeThinClawEvents(first);
        subscribeThinClawEvents(second);
        await vi.waitFor(() => expect(eventMock.callback).not.toBeNull());

        const payload: UiEvent = { kind: 'Connected', protocol: 1 };
        eventMock.callback?.({ payload });

        expect(eventMock.listen).toHaveBeenCalledTimes(1);
        expect(first).toHaveBeenCalledWith(payload);
        expect(second).toHaveBeenCalledWith(payload);

        unsubscribeFirst();
        eventMock.callback?.({ payload });
        expect(first).toHaveBeenCalledTimes(1);
        expect(second).toHaveBeenCalledTimes(2);
    });
});
