export const SCHEDULE_PRESETS = [
    { label: 'Every minute', value: '0 * * * * * *' },
    { label: 'Every 5 min', value: '0 */5 * * * * *' },
    { label: 'Every 15 min', value: '0 */15 * * * * *' },
    { label: 'Every hour', value: '0 0 * * * * *' },
    { label: 'Daily at 9am', value: '0 0 9 * * * *' },
    { label: 'Daily midnight', value: '0 0 0 * * * *' },
    { label: 'Weekly Mon', value: '0 0 9 * * 1 *' },
] as const;

export const INTERVAL_PRESETS = [
    { label: '5m', minutes: 5 },
    { label: '10m', minutes: 10 },
    { label: '15m', minutes: 15 },
    { label: '30m', minutes: 30 },
    { label: '1h', minutes: 60 },
    { label: '2h', minutes: 120 },
] as const;

export function parseIntervalMinutes(schedule: string | null | undefined): number {
    const value = schedule ?? '';
    const sevenField = value.match(/^0\s+\*\/(\d+)\s+\*\s+\*\s+\*\s+\*\s+\*$/);
    if (sevenField) return Number.parseInt(sevenField[1]!, 10);

    const fiveField = value.match(/^\*\/(\d+)\s+\*\s+\*\s+\*\s+\*$/);
    if (fiveField) return Number.parseInt(fiveField[1]!, 10);

    return 30;
}
