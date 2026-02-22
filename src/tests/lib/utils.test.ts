/**
 * Tests for src/lib/utils.ts
 *
 * Covers:
 *  - cn() — className merging helper
 *  - unwrap() — Result<T, string> extractor
 */

import { describe, it, expect } from 'vitest';
import { cn, unwrap } from '../../lib/utils';

// ---------------------------------------------------------------------------
// cn() — className merger (clsx + tailwind-merge)
// ---------------------------------------------------------------------------
describe('cn()', () => {
    it('returns a single class unchanged', () => {
        expect(cn('foo')).toBe('foo');
    });

    it('joins multiple classes', () => {
        expect(cn('a', 'b', 'c')).toBe('a b c');
    });

    it('ignores falsy values (undefined, false, null)', () => {
        expect(cn('a', undefined, false, null, 'b')).toBe('a b');
    });

    it('handles conditional class objects', () => {
        expect(cn({ active: true, hidden: false })).toBe('active');
    });

    it('merges conflicting Tailwind classes (last wins)', () => {
        // tailwind-merge resolves p-4 vs p-8 — p-8 should win
        expect(cn('p-4', 'p-8')).toBe('p-8');
    });

    it('merges conflicting text colours', () => {
        expect(cn('text-red-500', 'text-blue-500')).toBe('text-blue-500');
    });

    it('returns empty string when called with no args', () => {
        expect(cn()).toBe('');
    });
});

// ---------------------------------------------------------------------------
// unwrap() — Result<T, string> unwrapper
// ---------------------------------------------------------------------------
describe('unwrap()', () => {
    it('returns data when status is "ok"', () => {
        const result = { status: 'ok' as const, data: 42, error: '' };
        expect(unwrap(result)).toBe(42);
    });

    it('returns data for complex object payloads', () => {
        const payload = { id: '1', name: 'test' };
        const result = { status: 'ok' as const, data: payload, error: '' };
        expect(unwrap(result)).toEqual(payload);
    });

    it('throws when status is "error"', () => {
        const result = { status: 'error' as const, data: null as any, error: 'something broke' };
        expect(() => unwrap(result)).toThrowError('something broke');
    });

    it('throws with the exact error message from the result', () => {
        const msg = 'Config file not found';
        const result = { status: 'error' as const, data: null as any, error: msg };
        expect(() => unwrap(result)).toThrow(msg);
    });
});
