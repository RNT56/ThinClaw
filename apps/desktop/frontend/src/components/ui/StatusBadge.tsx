import { AlertTriangle, CheckCircle2, CircleDashed, CircleOff, Clock3, HelpCircle, LoaderCircle } from 'lucide-react';

import { cn } from '../../lib/utils';

export type StatusTone = 'healthy' | 'running' | 'warning' | 'error' | 'stopped' | 'unknown' | 'stale' | 'loading';

export interface StatusBadgeProps {
    status: string | null | undefined;
    label?: string;
    className?: string;
}

function normalizedTone(status: string | null | undefined): StatusTone {
    const value = status?.trim().toLowerCase() ?? '';
    if (['healthy', 'available', 'ready', 'connected', 'online', 'active', 'completed', 'success', 'applied', 'running'].includes(value)) {
        return value === 'running' || value === 'active' ? 'running' : 'healthy';
    }
    if (['loading', 'checking', 'connecting'].includes(value)) return 'loading';
    if (['degraded', 'partial', 'waiting', 'pending', 'restart required', 'restart-required', 'stale'].includes(value)) {
        return value === 'stale' ? 'stale' : 'warning';
    }
    if (['failed', 'error', 'denied', 'disconnected', 'unavailable'].includes(value)) return 'error';
    if (['stopped', 'offline', 'disabled', 'inactive'].includes(value)) return 'stopped';
    return 'unknown';
}

const toneClass: Record<StatusTone, string> = {
    healthy: 'border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300',
    running: 'border-primary/30 bg-primary/10 text-primary',
    warning: 'border-amber-500/30 bg-amber-500/10 text-amber-800 dark:text-amber-300',
    error: 'border-destructive/30 bg-destructive/10 text-destructive',
    stopped: 'border-surface-outline bg-surface-subtle text-content-muted',
    unknown: 'border-surface-outline bg-surface-subtle text-content-muted',
    stale: 'border-amber-500/30 bg-amber-500/10 text-amber-800 dark:text-amber-300',
    loading: 'border-surface-outline bg-surface-subtle text-content-muted',
};

const toneIcon: Record<StatusTone, typeof CheckCircle2> = {
    healthy: CheckCircle2,
    running: CircleDashed,
    warning: AlertTriangle,
    error: CircleOff,
    stopped: CircleOff,
    unknown: HelpCircle,
    stale: Clock3,
    loading: LoaderCircle,
};

export function StatusBadge({ status, label, className }: StatusBadgeProps) {
    const tone = normalizedTone(status);
    const Icon = toneIcon[tone];
    const visibleLabel = label ?? status ?? 'Unknown';
    return (
        <span className={cn('inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[10px] font-semibold', toneClass[tone], className)}>
            <Icon className={cn('size-3', tone === 'loading' && 'animate-spin')} aria-hidden="true" />
            {visibleLabel}
        </span>
    );
}
