import { describe, expect, it } from 'vitest';

import {
    extractSessionKey,
    formatToolValue,
    safeExternalUrl,
} from '../../components/thinclaw/ChatSubComponents';

describe('untrusted tool output boundaries', () => {
    it('accepts bounded session identifiers and rejects objects, controls, and oversized values', () => {
        expect(extractSessionKey({ session_key: 'agent:worker-1' })).toBe('agent:worker-1');
        expect(extractSessionKey({ session_key: { attacker: true } })).toBeNull();
        expect(extractSessionKey({ session_key: 'agent:worker\nspoofed' })).toBeNull();
        expect(extractSessionKey({ session_key: `agent:${'a'.repeat(300)}` })).toBeNull();
    });

    it('formats objects safely and truncates oversized runtime output', () => {
        expect(formatToolValue({ error: { message: 'safe text' } })).toContain('safe text');
        const circular: { self?: unknown } = {};
        circular.self = circular;
        expect(formatToolValue(circular)).toBe('[Unserializable tool output]');
        expect(formatToolValue('x'.repeat(100_001))).toContain('[output truncated]');
    });

    it('opens only bounded credential-free HTTP(S) URLs', () => {
        expect(safeExternalUrl('https://example.com/docs')).toBe('https://example.com/docs');
        expect(safeExternalUrl('javascript:alert(1)')).toBeNull();
        expect(safeExternalUrl('https://user:password@example.com')).toBeNull();
        expect(safeExternalUrl(`https://example.com/${'x'.repeat(2_100)}`)).toBeNull();
    });
});
