import { describe, expect, it } from 'vitest';

const frontendSources = import.meta.glob('../../**/*.{ts,tsx}', {
    eager: true,
    query: '?raw',
    import: 'default',
}) as Record<string, string>;

function productionSources(): string[] {
    return Object.entries(frontendSources)
        .filter(([path]) => !path.includes('/tests/'))
        .map(([, source]) => source);
}

describe('ThinClaw event bus migration', () => {
    it('does not leave frontend listeners on the legacy OpenClaw bus', () => {
        const matches = productionSources().filter(source => source.includes('openclaw-event'));

        expect(matches).toEqual([]);
    });

    it('keeps Desktop event consumers on thinclaw-event', () => {
        const listenerCount = productionSources()
            .map(source => source.match(/['"]thinclaw-event['"]/g)?.length ?? 0)
            .reduce((sum, count) => sum + count, 0);

        expect(listenerCount).toBeGreaterThanOrEqual(10);
    });
});
