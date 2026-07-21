import { useCallback, useEffect, useState } from 'react';
import {
    AlertTriangle,
    CheckCircle2,
    Clipboard,
    Globe2,
    Loader2,
    LockKeyhole,
    Network,
    RefreshCw,
    ShieldCheck,
    Square,
} from 'lucide-react';
import { toast } from 'sonner';
import * as thinclaw from '../../lib/thinclaw';
import { cn } from '../../lib/utils';

type BusyAction = 'refresh' | 'start' | 'stop' | null;

function ReadinessRow({
    ready,
    title,
    detail,
}: {
    ready: boolean;
    title: string;
    detail: string;
}) {
    return (
        <div className="flex items-start gap-3 rounded-xl border border-border/40 bg-background/35 p-4">
            {ready
                ? <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0 text-emerald-400" />
                : <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-amber-400" />}
            <div className="min-w-0">
                <p className="text-xs font-semibold text-foreground">{title}</p>
                <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">{detail}</p>
            </div>
        </div>
    );
}

export function ThinClawRemoteAccess() {
    const [status, setStatus] = useState<thinclaw.RemoteAccessStatus | null>(null);
    const [exposure, setExposure] = useState<thinclaw.RemoteAccessExposure>('tailnet');
    const [confirmPublic, setConfirmPublic] = useState(false);
    const [busy, setBusy] = useState<BusyAction>('refresh');
    const [error, setError] = useState<string | null>(null);

    const refresh = useCallback(async (showSpinner = true) => {
        if (showSpinner) setBusy('refresh');
        try {
            const next = await thinclaw.getRemoteAccessStatus();
            setStatus(next);
            if (next.exposure) setExposure(next.exposure);
            setError(null);
        } catch (caught) {
            setError(String(caught));
        } finally {
            if (showSpinner) setBusy(null);
        }
    }, []);

    useEffect(() => {
        void refresh();
    }, [refresh]);

    const start = async () => {
        setBusy('start');
        try {
            const next = await thinclaw.startRemoteAccess(exposure, confirmPublic);
            setStatus(next);
            setError(null);
            toast.success(exposure === 'tailnet'
                ? 'Private tailnet access is active'
                : 'Public Tailscale Funnel is active');
        } catch (caught) {
            const message = String(caught);
            setError(message);
            toast.error(message);
        } finally {
            setBusy(null);
        }
    };

    const stop = async () => {
        setBusy('stop');
        try {
            const next = await thinclaw.stopRemoteAccess();
            setStatus(next);
            setError(null);
            toast.success('Remote access stopped');
        } catch (caught) {
            const message = String(caught);
            setError(message);
            toast.error(message);
        } finally {
            setBusy(null);
        }
    };

    const copyUrl = async () => {
        if (!status?.access_url) return;
        await navigator.clipboard.writeText(status.access_url);
        toast.success('Access URL copied');
    };

    const ready = Boolean(status?.gateway_running && status?.tailscale_authenticated);

    return (
        <main className="flex-1 overflow-y-auto p-6" aria-labelledby="remote-access-title">
            <div className="mx-auto max-w-5xl space-y-5">
                <header className="flex flex-wrap items-start justify-between gap-4">
                    <div>
                        <div className="flex items-center gap-2">
                            <Network className="h-5 w-5 text-primary" />
                            <h1 id="remote-access-title" className="text-xl font-bold">Remote Access</h1>
                        </div>
                        <p className="mt-1 max-w-2xl text-sm text-muted-foreground">
                            Expose this Desktop&apos;s authenticated ThinClaw gateway through Tailscale. Private tailnet access is the safe default; public Funnel is an explicit internet boundary.
                        </p>
                    </div>
                    <button
                        type="button"
                        onClick={() => void refresh()}
                        disabled={busy !== null}
                        className="inline-flex items-center gap-2 rounded-lg border border-border/50 px-3 py-2 text-xs font-semibold text-muted-foreground transition-colors hover:text-foreground disabled:opacity-50"
                    >
                        <RefreshCw className={cn('h-3.5 w-3.5', busy === 'refresh' && 'animate-spin')} />
                        Refresh
                    </button>
                </header>

                <section className="grid gap-3 md:grid-cols-2" aria-label="Remote access readiness">
                    <ReadinessRow
                        ready={Boolean(status?.gateway_running)}
                        title="Authenticated loopback gateway"
                        detail={status?.gateway_running
                            ? `Listening privately at ${status.gateway_url}`
                            : status?.runtime_mode === 'remote'
                                ? 'Switch to Local Core. Remote hosts manage their own exposure.'
                                : 'Start Local Core in Gateway settings to mount the loopback listener.'}
                    />
                    <ReadinessRow
                        ready={Boolean(status?.tailscale_authenticated)}
                        title="Tailscale CLI and session"
                        detail={status?.tailscale_authenticated
                            ? `Signed in as ${status.tailscale_dns_name ?? 'this tailnet device'}`
                            : status?.tailscale_error ?? 'Checking whether Tailscale is installed and signed in…'}
                    />
                </section>

                {error && (
                    <div role="alert" className="rounded-xl border border-red-500/25 bg-red-500/10 p-4 text-xs leading-relaxed text-red-200">
                        {error}
                    </div>
                )}

                {status?.tunnel_running ? (
                    <section className="rounded-2xl border border-emerald-500/25 bg-emerald-500/7 p-5">
                        <div className="flex flex-wrap items-start justify-between gap-4">
                            <div className="flex items-start gap-3">
                                <div className="rounded-xl bg-emerald-500/15 p-2.5 text-emerald-300">
                                    {status.exposure === 'public' ? <Globe2 className="h-5 w-5" /> : <ShieldCheck className="h-5 w-5" />}
                                </div>
                                <div>
                                    <p className="text-sm font-bold text-emerald-100">
                                        {status.exposure === 'public' ? 'Public Funnel active' : 'Private tailnet access active'}
                                    </p>
                                    <p className="mt-1 break-all font-mono text-xs text-emerald-200/80">{status.access_url}</p>
                                    <p className="mt-2 text-[11px] text-muted-foreground">
                                        Gateway authentication still applies. Use the credential from Gateway settings when connecting a client.
                                    </p>
                                </div>
                            </div>
                            <div className="flex items-center gap-2">
                                <button
                                    type="button"
                                    onClick={() => void copyUrl()}
                                    className="inline-flex items-center gap-2 rounded-lg border border-emerald-500/25 px-3 py-2 text-xs font-semibold text-emerald-100 hover:bg-emerald-500/10"
                                >
                                    <Clipboard className="h-3.5 w-3.5" />
                                    Copy URL
                                </button>
                                <button
                                    type="button"
                                    onClick={() => void stop()}
                                    disabled={busy !== null}
                                    className="inline-flex items-center gap-2 rounded-lg bg-red-500/15 px-3 py-2 text-xs font-semibold text-red-200 hover:bg-red-500/20 disabled:opacity-50"
                                >
                                    {busy === 'stop' ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Square className="h-3.5 w-3.5" />}
                                    Stop
                                </button>
                            </div>
                        </div>
                    </section>
                ) : (
                    <section className="rounded-2xl border border-border/50 bg-card/35 p-5">
                        <div className="grid gap-4 lg:grid-cols-2">
                            <button
                                type="button"
                                onClick={() => {
                                    setExposure('tailnet');
                                    setConfirmPublic(false);
                                }}
                                className={cn(
                                    'rounded-xl border p-4 text-left transition-colors',
                                    exposure === 'tailnet'
                                        ? 'border-primary/45 bg-primary/8'
                                        : 'border-border/45 bg-background/25 hover:border-border',
                                )}
                            >
                                <div className="flex items-center gap-2 text-sm font-bold">
                                    <LockKeyhole className="h-4 w-4 text-emerald-400" />
                                    Tailnet only
                                    <span className="rounded-full bg-emerald-500/10 px-2 py-0.5 text-[9px] uppercase tracking-wide text-emerald-300">Recommended</span>
                                </div>
                                <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                                    Uses Tailscale Serve. Only devices signed into your tailnet can reach the gateway.
                                </p>
                            </button>
                            <button
                                type="button"
                                onClick={() => setExposure('public')}
                                className={cn(
                                    'rounded-xl border p-4 text-left transition-colors',
                                    exposure === 'public'
                                        ? 'border-amber-500/45 bg-amber-500/8'
                                        : 'border-border/45 bg-background/25 hover:border-border',
                                )}
                            >
                                <div className="flex items-center gap-2 text-sm font-bold">
                                    <Globe2 className="h-4 w-4 text-amber-400" />
                                    Public Funnel
                                </div>
                                <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
                                    Uses Tailscale Funnel for public HTTPS callbacks. Anyone can reach the login boundary, so keep the gateway token private.
                                </p>
                            </button>
                        </div>

                        {exposure === 'public' && (
                            <label className="mt-4 flex cursor-pointer items-start gap-3 rounded-xl border border-amber-500/25 bg-amber-500/8 p-4">
                                <input
                                    type="checkbox"
                                    checked={confirmPublic}
                                    onChange={event => setConfirmPublic(event.target.checked)}
                                    className="mt-0.5 h-4 w-4 accent-amber-500"
                                />
                                <span className="text-xs leading-relaxed text-amber-100">
                                    I understand this creates a public internet endpoint and gateway authentication remains mandatory.
                                </span>
                            </label>
                        )}

                        <div className="mt-5 flex flex-wrap items-center justify-between gap-3 border-t border-border/40 pt-4">
                            <p className="max-w-xl text-[11px] leading-relaxed text-muted-foreground">
                                ThinClaw never accepts a Tailscale auth key here. Install Tailscale and sign in through its own app or CLI; Desktop only controls Serve/Funnel lifecycle.
                            </p>
                            <button
                                type="button"
                                onClick={() => void start()}
                                disabled={!ready || busy !== null || (exposure === 'public' && !confirmPublic)}
                                className="inline-flex items-center gap-2 rounded-lg bg-primary px-4 py-2.5 text-xs font-bold text-primary-foreground shadow-sm transition-opacity disabled:cursor-not-allowed disabled:opacity-40"
                            >
                                {busy === 'start' ? <Loader2 className="h-4 w-4 animate-spin" /> : <Network className="h-4 w-4" />}
                                Enable {exposure === 'public' ? 'public Funnel' : 'tailnet access'}
                            </button>
                        </div>
                    </section>
                )}
            </div>
        </main>
    );
}
