import { describe, expect, it } from 'vitest';

import { PAIRING_CHANNELS } from '../../components/thinclaw/pairing/catalog';

describe('pairing channel catalog', () => {
    it('matches the adapters that enforce the shared pairing store', () => {
        expect(PAIRING_CHANNELS.map((channel) => channel.id)).toEqual([
            'telegram',
            'slack',
            'discord',
            'whatsapp',
            'signal',
        ]);
    });

    it('describes credential setup instead of offering unsupported web login', () => {
        expect(PAIRING_CHANNELS.every((channel) => channel.setup.length > 0)).toBe(true);
        expect(PAIRING_CHANNELS.some((channel) => /web login/i.test(channel.setup))).toBe(false);
    });
});
