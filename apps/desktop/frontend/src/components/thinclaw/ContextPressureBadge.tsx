import { AlertTriangle } from 'lucide-react';

import { cn } from '../../lib/utils';

export type ContextPressureLevel = 'none' | 'warning' | 'critical';

interface ContextPressureBadgeProps {
    level: ContextPressureLevel;
    usagePercent: number;
}

export function ContextPressureBadge({ level, usagePercent }: ContextPressureBadgeProps) {
    if (level === 'none') return null;

    const boundedUsage = Number.isFinite(usagePercent)
        ? Math.max(0, Math.min(999.9, usagePercent))
        : 0;
    const roundedUsage = Math.round(boundedUsage);
    const critical = level === 'critical';
    const label = `Context window ${level}: ${roundedUsage}% used`;

    return (
        <span
            role="status"
            aria-label={label}
            title={critical
                ? `${label}. Automatic compaction is imminent.`
                : `${label}. Consider compacting or starting a new thread.`}
            className={cn(
                'inline-flex items-center gap-1 rounded-md border px-2 py-1 text-[10px] font-semibold tabular-nums',
                critical
                    ? 'border-red-500/30 bg-red-500/10 text-red-400'
                    : 'border-amber-500/30 bg-amber-500/10 text-amber-400',
            )}
        >
            <AlertTriangle className="h-3 w-3" aria-hidden="true" />
            <span>{roundedUsage}% context</span>
        </span>
    );
}
