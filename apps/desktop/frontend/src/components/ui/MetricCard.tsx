import { type ReactNode } from 'react';

import { cn } from '../../lib/utils';
import { Surface } from './Surface';

export interface MetricCardProps {
    label: string;
    value: ReactNode;
    detail?: ReactNode;
    action?: ReactNode;
    className?: string;
}

/** A metric card that always carries context rather than decorative numbers. */
export function MetricCard({ label, value, detail, action, className }: MetricCardProps) {
    return (
        <Surface className={cn('min-w-0 p-4', className)}>
            <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                    <p className="text-[10px] font-bold uppercase tracking-widest text-content-muted">{label}</p>
                    <p className="mt-1 truncate text-2xl font-semibold tabular-nums text-content-primary">{value}</p>
                    {detail && <div className="mt-1 text-xs leading-relaxed text-content-muted">{detail}</div>}
                </div>
                {action}
            </div>
        </Surface>
    );
}
