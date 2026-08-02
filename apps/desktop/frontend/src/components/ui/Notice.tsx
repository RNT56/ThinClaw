import { AlertCircle, AlertTriangle, CheckCircle2, Info } from 'lucide-react';
import { type ReactNode } from 'react';

import { cn } from '../../lib/utils';

export type NoticeTone = 'info' | 'warning' | 'error' | 'success';

export interface NoticeProps {
    tone?: NoticeTone;
    title?: string;
    children: ReactNode;
    className?: string;
}

const noticeStyles: Record<NoticeTone, { container: string; icon: typeof Info }> = {
    info: { container: 'border-primary/25 bg-primary/8 text-content-primary', icon: Info },
    warning: { container: 'border-amber-500/30 bg-amber-500/10 text-content-primary', icon: AlertTriangle },
    error: { container: 'border-destructive/30 bg-destructive/10 text-content-primary', icon: AlertCircle },
    success: { container: 'border-emerald-500/30 bg-emerald-500/10 text-content-primary', icon: CheckCircle2 },
};

export function Notice({ tone = 'info', title, children, className }: NoticeProps) {
    const { container, icon: Icon } = noticeStyles[tone];
    return (
        <div role={tone === 'error' ? 'alert' : 'status'} className={cn('flex gap-3 rounded-[var(--radius-control)] border p-4', container, className)}>
            <Icon className="mt-0.5 size-4 shrink-0" aria-hidden="true" />
            <div className="min-w-0 text-xs leading-relaxed">
                {title && <p className="font-semibold">{title}</p>}
                <div className={cn(title && 'mt-1', 'text-content-muted')}>{children}</div>
            </div>
        </div>
    );
}
