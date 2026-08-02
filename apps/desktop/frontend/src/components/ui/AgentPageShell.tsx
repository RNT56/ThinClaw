import { type ReactNode } from 'react';

import { cn } from '../../lib/utils';

export interface AgentPageShellProps {
    title: string;
    description: string;
    eyebrow?: string;
    badge?: ReactNode;
    actions?: ReactNode;
    children: ReactNode;
    className?: string;
    contentClassName?: string;
}

/** Shared frame for every rebuilt Agent Cockpit destination. */
export function AgentPageShell({
    title,
    description,
    eyebrow,
    badge,
    actions,
    children,
    className,
    contentClassName,
}: AgentPageShellProps) {
    return (
        <main className={cn('flex min-h-0 flex-1 flex-col overflow-hidden bg-surface-canvas text-content-primary', className)}>
            <header className="shrink-0 border-b border-surface-outline px-6 py-5">
                <div className="mx-auto flex w-full max-w-7xl flex-wrap items-start justify-between gap-4">
                    <div className="min-w-0">
                        {eyebrow && <p className="text-[10px] font-bold uppercase tracking-widest text-content-muted">{eyebrow}</p>}
                        <div className="mt-1 flex flex-wrap items-center gap-2">
                            <h1 className="text-xl font-semibold">{title}</h1>
                            {badge}
                        </div>
                        <p className="mt-1 max-w-3xl text-sm text-content-muted">{description}</p>
                    </div>
                    {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
                </div>
            </header>
            <div className={cn('min-h-0 flex-1 overflow-y-auto px-6 py-5', contentClassName)}>{children}</div>
        </main>
    );
}
