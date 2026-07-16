import { useCallback, useEffect, useState } from 'react';
import { AlertTriangle, BrainCircuit, Clock, Play, RefreshCw } from 'lucide-react';
import { toast } from 'sonner';

import * as thinclaw from '../../../lib/thinclaw';

function timestamp(value: string | null): string {
    return value ? new Date(value).toLocaleString() : 'Never';
}

export function ProfileEvolutionPanel() {
    const [status, setStatus] = useState<thinclaw.ProfileEvolutionStatus | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [loading, setLoading] = useState(true);
    const [running, setRunning] = useState(false);

    const load = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            setStatus(await thinclaw.getProfileEvolutionStatus());
        } catch (loadError) {
            setError(String(loadError));
        } finally {
            setLoading(false);
        }
    }, []);

    useEffect(() => {
        load();
    }, [load]);

    const runNow = async () => {
        setRunning(true);
        try {
            await thinclaw.runProfileEvolution();
            toast.success('Profile evolution started');
            await load();
        } catch (runError) {
            toast.error(`Profile evolution failed: ${String(runError)}`);
        } finally {
            setRunning(false);
        }
    };

    return (
        <section className="rounded-xl border border-border/40 bg-card/30 p-5">
            <div className="flex items-start justify-between gap-4">
                <div className="flex items-start gap-3">
                    <div className="rounded-lg border border-primary/20 bg-primary/10 p-2">
                        <BrainCircuit className="h-4 w-4 text-primary" />
                    </div>
                    <div>
                        <div className="flex items-center gap-2">
                            <h2 className="text-sm font-bold">Profile Evolution</h2>
                            {status && (
                                <span className={`rounded-full px-2 py-0.5 text-[9px] font-bold uppercase ${status.routine_enabled ? 'bg-emerald-500/10 text-emerald-300' : 'bg-amber-500/10 text-amber-300'}`}>
                                    {status.routine_enabled ? 'scheduled' : 'not scheduled'}
                                </span>
                            )}
                        </div>
                        <p className="mt-1 text-[11px] text-muted-foreground">
                            Conservative weekly updates derived from first-party conversation evidence.
                        </p>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    <button
                        aria-label="Refresh profile evolution"
                        onClick={load}
                        disabled={loading || running}
                        className="rounded-lg border border-white/5 bg-white/3 p-2 text-muted-foreground transition-all hover:bg-white/5 hover:text-foreground disabled:opacity-40"
                    >
                        <RefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />
                    </button>
                    <button
                        onClick={runNow}
                        disabled={loading || running}
                        className="inline-flex items-center gap-1.5 rounded-lg border border-primary/20 bg-primary/10 px-3 py-2 text-[11px] font-semibold text-primary transition-all hover:bg-primary/15 disabled:opacity-40"
                    >
                        {running ? <RefreshCw className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
                        Run evolution now
                    </button>
                </div>
            </div>

            {error ? (
                <div className="mt-4 flex items-start gap-2 rounded-lg border border-amber-500/20 bg-amber-500/10 p-3 text-xs text-amber-200">
                    <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                    <span>{error}</span>
                </div>
            ) : status ? (
                <div className="mt-4 space-y-4">
                    <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
                        <div className="rounded-lg border border-border/30 bg-white/2 p-3">
                            <div className="text-[9px] font-bold uppercase tracking-wider text-muted-foreground">User</div>
                            <div className="mt-1 truncate text-sm font-semibold">{status.preferred_name || 'Not learned yet'}</div>
                        </div>
                        <div className="rounded-lg border border-border/30 bg-white/2 p-3">
                            <div className="text-[9px] font-bold uppercase tracking-wider text-muted-foreground">Confidence</div>
                            <div className="mt-1 text-sm font-semibold tabular-nums">{status.confidence == null ? '—' : `${Math.round(status.confidence * 100)}%`}</div>
                        </div>
                        <div className="rounded-lg border border-border/30 bg-white/2 p-3">
                            <div className="text-[9px] font-bold uppercase tracking-wider text-muted-foreground">Evidence</div>
                            <div className="mt-1 text-sm font-semibold tabular-nums">{status.message_count == null ? '—' : `${status.message_count} messages`}</div>
                        </div>
                        <div className="rounded-lg border border-border/30 bg-white/2 p-3">
                            <div className="text-[9px] font-bold uppercase tracking-wider text-muted-foreground">Runs</div>
                            <div className="mt-1 text-sm font-semibold tabular-nums">{status.run_count}</div>
                        </div>
                    </div>

                    <div className="flex flex-wrap gap-x-5 gap-y-2 text-[10px] text-muted-foreground">
                        <span className="flex items-center gap-1.5"><Clock className="h-3 w-3" />Next: {timestamp(status.next_fire_at)}</span>
                        <span>Last run: {timestamp(status.last_run_at)}</span>
                        <span>Profile updated: {timestamp(status.profile_updated_at)}</span>
                        {status.consecutive_failures > 0 && <span className="text-red-300">{status.consecutive_failures} consecutive failures</span>}
                    </div>

                    {status.profile_parse_error && (
                        <div className="rounded-lg border border-red-500/20 bg-red-500/10 p-3 text-xs text-red-300">{status.profile_parse_error}</div>
                    )}
                    {!status.profile_exists && (
                        <div className="rounded-lg border border-dashed border-border/40 p-3 text-xs text-muted-foreground">
                            No profile exists yet. Add durable user context in USER.md before running evolution.
                        </div>
                    )}
                    {status.profile && (
                        <details className="rounded-lg border border-border/30 bg-black/10">
                            <summary className="cursor-pointer px-3 py-2 text-[10px] font-bold uppercase tracking-wider text-muted-foreground">View current profile JSON</summary>
                            <pre className="max-h-80 overflow-auto border-t border-border/30 p-3 text-[10px] leading-relaxed text-zinc-300">{JSON.stringify(status.profile, null, 2)}</pre>
                        </details>
                    )}
                </div>
            ) : null}
        </section>
    );
}
