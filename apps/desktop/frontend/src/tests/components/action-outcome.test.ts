import { describe, expect, it } from 'vitest';

import { normalizeAgentActionOutcome } from '../../components/thinclaw/action-outcome';

describe('normalizeAgentActionOutcome', () => {
    it('does not confuse persisted-only or restart-required responses with applied success', () => {
        expect(normalizeAgentActionOutcome({ ok: false, persisted: true, forwarded: false, note: 'Saved for next start' }, 'fallback'))
            .toEqual({ state: 'persisted', message: 'Saved for next start' });
        expect(normalizeAgentActionOutcome({ ok: true, restart_required: true, note: 'Restart required' }, 'fallback'))
            .toEqual({ state: 'restart-required', message: 'Restart required' });
    });

    it('keeps an unknown transport payload unknown', () => {
        expect(normalizeAgentActionOutcome({}, 'No outcome supplied')).toEqual({ state: 'unknown', message: 'No outcome supplied' });
    });
});
