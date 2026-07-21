import { describe, expect, it } from 'vitest';

import {
    INTERVAL_PRESETS,
    SCHEDULE_PRESETS,
    parseIntervalMinutes,
} from '../../components/thinclaw/automations/schedule';

describe('automation schedule helpers', () => {
    it('parses heartbeat intervals from both supported cron shapes', () => {
        expect(parseIntervalMinutes('0 */15 * * * * *')).toBe(15);
        expect(parseIntervalMinutes('*/60 * * * *')).toBe(60);
        expect(parseIntervalMinutes('0 0 9 * * * *')).toBe(30);
    });

    it('keeps preset values unique and selectable', () => {
        expect(new Set(SCHEDULE_PRESETS.map((preset) => preset.value)).size).toBe(
            SCHEDULE_PRESETS.length,
        );
        expect(new Set(INTERVAL_PRESETS.map((preset) => preset.minutes)).size).toBe(
            INTERVAL_PRESETS.length,
        );
    });
});
