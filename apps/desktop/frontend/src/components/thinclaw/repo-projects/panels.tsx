import type { ReactNode } from 'react';
import type { LucideIcon } from 'lucide-react';

import { cn } from '../../../lib/utils';
import { stateTone, statusLabel } from './utils';

export function StateBadge({ state, label }: { state?: string; label?: string }) {
    return (
        <span className={cn('inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider', stateTone(state))}>
            {label ?? statusLabel(state)}
        </span>
    );
}
export function SectionCard({
    title,
    icon: Icon,
    children,
    action,
}: {
    title: string;
    icon: LucideIcon;
    children: ReactNode;
    action?: ReactNode;
}) {
    return (
        <div className="rounded-lg border border-border/40 bg-card/30 p-5">
            <div className="mb-4 flex items-center justify-between gap-3">
                <div className="flex items-center gap-2">
                    <Icon className="h-4 w-4 text-primary" />
                    <h3 className="text-sm font-bold">{title}</h3>
                </div>
                {action}
            </div>
            {children}
        </div>
    );
}

export function MetricCard({ label, value, tone }: { label: string; value: string | number; tone?: string }) {
    return (
        <div className="rounded-lg border border-border/40 bg-card/30 p-4">
            <p className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground">{label}</p>
            <p className={cn('mt-1 text-2xl font-bold tabular-nums', tone)}>{value}</p>
        </div>
    );
}
