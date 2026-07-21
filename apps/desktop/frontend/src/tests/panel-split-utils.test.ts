import { describe, expect, it } from 'vitest';
import {
    CHANNEL_DESCRIPTIONS,
    STREAM_MODE_LABELS,
    STREAM_MODES,
    channelIcon
} from '../components/thinclaw/channels/catalog';
import { actionOk, normalizeSkill } from '../components/thinclaw/skills/utils';

describe('channel catalog', () => {
    it('keeps every runtime stream mode labelled', () => {
        expect(STREAM_MODES.every((mode) => Boolean(STREAM_MODE_LABELS[mode]))).toBe(true);
        expect(CHANNEL_DESCRIPTIONS.gmail).toContain('OAuth 2.0');
        expect(channelIcon('unknown')).toBeDefined();
    });
});

describe('skill transport normalization', () => {
    it('normalizes legacy skill keys and response envelopes', () => {
        expect(normalizeSkill({ key: 'shell', disabled: true })).toMatchObject({
            skillKey: 'shell',
            name: 'shell',
            disabled: true,
            eligible: true
        });
        expect(actionOk({ success: true })).toBe(true);
        expect(actionOk({ ok: false })).toBe(false);
    });
});
