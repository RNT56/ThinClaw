import { useEffect, useState } from 'react';
import { Cloud, Laptop, AlertTriangle, RefreshCw } from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';

export function isRemoteOpenClaw(status: openclaw.OpenClawStatus | null | undefined): boolean {
    return (status?.gateway_mode || '').toLowerCase() === 'remote';
}

export function modeLabel(status: openclaw.OpenClawStatus | null | undefined): string {
    if (!status) return 'Checking';
    if (isRemoteOpenClaw(status)) return status.remote_url ? 'Remote gateway' : 'Remote not configured';
    return 'Local runtime';
}

export function OpenClawModeBadge({
    status,
    compact = false,
    className,
}: {
    status: openclaw.OpenClawStatus | null | undefined;
    compact?: boolean;
    className?: string;
}) {
    const remote = isRemoteOpenClaw(status);
    const configured = !remote || !!status?.remote_url;
    const Icon = !status ? RefreshCw : remote ? Cloud : Laptop;

    return (
        <span
            className={cn(
                'inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[10px] font-bold uppercase tracking-wider',
                !status
                    ? 'border-border/40 bg-muted/30 text-muted-foreground'
                    : !configured
                        ? 'border-amber-500/25 bg-amber-500/10 text-amber-400'
                        : remote
                            ? 'border-cyan-500/25 bg-cyan-500/10 text-cyan-400'
                            : 'border-emerald-500/25 bg-emerald-500/10 text-emerald-400',
                className,
            )}
            title={remote && status?.remote_url ? status.remote_url : modeLabel(status)}
        >
            {!configured ? <AlertTriangle className="h-3 w-3" /> : <Icon className={cn('h-3 w-3', !status && 'animate-spin')} />}
            {!compact && modeLabel(status)}
        </span>
    );
}

export function useOpenClawStatusSnapshot(intervalMs = 15000) {
    const [status, setStatus] = useState<openclaw.OpenClawStatus | null>(null);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        let cancelled = false;
        const load = async () => {
            try {
                const next = await openclaw.getOpenClawStatus();
                if (!cancelled) {
                    setStatus(next);
                    setError(null);
                }
            } catch (err) {
                if (!cancelled) setError(String(err));
            }
        };
        load();
        const timer = intervalMs > 0 ? window.setInterval(load, intervalMs) : null;
        return () => {
            cancelled = true;
            if (timer) window.clearInterval(timer);
        };
    }, [intervalMs]);

    return { status, error, isRemote: isRemoteOpenClaw(status) };
}
