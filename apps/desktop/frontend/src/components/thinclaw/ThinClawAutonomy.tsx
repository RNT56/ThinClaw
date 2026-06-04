import { useCallback, useEffect, useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import {
    AlertTriangle,
    CheckCircle2,
    ClipboardCheck,
    FileSearch,
    Pause,
    Play,
    RefreshCw,
    RotateCcw,
    Shield,
    TerminalSquare,
    XCircle,
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';

function errorText(err: unknown) {
    return err instanceof Error ? err.message : String(err);
}

function JsonBlock({ value }: { value: unknown }) {
    return (
        <pre className="max-h-64 overflow-auto rounded-lg bg-black/20 border border-white/5 p-3 text-[11px] text-muted-foreground whitespace-pre-wrap">
            {value == null ? 'No data' : JSON.stringify(value, null, 2)}
        </pre>
    );
}

function CheckRow({ check }: { check: thinclaw.AutonomyCheckResult }) {
    return (
        <div className="flex items-start gap-3 rounded-lg border border-white/5 bg-black/10 p-3">
            {check.passed ? (
                <CheckCircle2 className="w-4 h-4 text-emerald-400 mt-0.5 shrink-0" />
            ) : (
                <XCircle className="w-4 h-4 text-red-400 mt-0.5 shrink-0" />
            )}
            <div className="min-w-0">
                <p className="text-sm font-semibold">{check.name}</p>
                {check.detail && <p className="text-xs text-muted-foreground mt-1">{check.detail}</p>}
            </div>
        </div>
    );
}

export function ThinClawAutonomy() {
    const [status, setStatus] = useState<thinclaw.AutonomyStatus | null>(null);
    const [permissions, setPermissions] = useState<unknown>(null);
    const [rollouts, setRollouts] = useState<thinclaw.AutonomyRolloutSummary | null>(null);
    const [checks, setChecks] = useState<thinclaw.AutonomyChecksSummary | null>(null);
    const [evidence, setEvidence] = useState<thinclaw.AutonomyEvidenceSummary | null>(null);
    const [lastBootstrap, setLastBootstrap] = useState<unknown>(null);
    const [error, setError] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [isMutating, setIsMutating] = useState(false);

    const load = useCallback(async () => {
        setIsLoading(true);
        setError(null);
        const errors: string[] = [];
        const [
            nextStatus,
            nextPermissions,
            nextRollouts,
            nextChecks,
            nextEvidence,
        ] = await Promise.all([
            thinclaw.getAutonomyStatus().catch((err) => {
                errors.push(errorText(err));
                return null;
            }),
            thinclaw.getAutonomyPermissions().catch((err) => ({ unavailable: errorText(err) })),
            thinclaw.getAutonomyRollouts().catch((err) => {
                errors.push(errorText(err));
                return null;
            }),
            thinclaw.getAutonomyChecks().catch((err) => {
                errors.push(errorText(err));
                return null;
            }),
            thinclaw.getAutonomyEvidence().catch((err) => {
                errors.push(errorText(err));
                return null;
            }),
        ]);
        setStatus(nextStatus);
        setPermissions(nextPermissions);
        setRollouts(nextRollouts);
        setChecks(nextChecks);
        setEvidence(nextEvidence);
        setError(errors[0] ?? null);
        setIsLoading(false);
    }, []);

    useEffect(() => {
        load();
        const interval = setInterval(load, 15000);
        return () => clearInterval(interval);
    }, [load]);

    const gatedReason = useMemo(() => {
        if (!status) return error ?? 'Autonomy status is unavailable.';
        if (!status.enabled) return 'Desktop autonomy is disabled by host policy.';
        if (status.emergency_stop_active) return 'Emergency stop is active.';
        if (!status.bootstrap_passed) return 'Bootstrap has not passed.';
        if (!status.action_ready) return status.blocking_reason ?? 'Host action prerequisites are not ready.';
        return null;
    }, [status, error]);

    const canMutate = Boolean(status?.enabled && !status.emergency_stop_active);

    const runMutation = async (label: string, action: () => Promise<unknown>) => {
        setIsMutating(true);
        try {
            const result = await action();
            if (label === 'Bootstrap') setLastBootstrap(result);
            toast.success(`${label} submitted`);
            await load();
        } catch (err) {
            toast.error(errorText(err));
        } finally {
            setIsMutating(false);
        }
    };

    const allChecks = [
        ...(checks?.bootstrap_checks ?? []),
        ...(checks?.latest_canary_checks ?? []),
    ];

    return (
        <motion.div className="flex-1 overflow-y-auto p-8 space-y-6" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
            <div className="flex items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-lg bg-emerald-500/10 border border-emerald-500/20">
                        <Shield className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Autonomy</h1>
                        <p className="text-xs text-muted-foreground">Status, host permissions, rollouts, checks, evidence, and gated controls</p>
                    </div>
                </div>
                <button
                    onClick={load}
                    className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                >
                    <RefreshCw className={cn('w-4 h-4', isLoading && 'animate-spin')} />
                </button>
            </div>

            {(error || gatedReason) && (
                <div className="flex items-start gap-3 rounded-lg border border-amber-500/20 bg-amber-500/10 p-4 text-sm text-amber-200">
                    <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" />
                    <span>{gatedReason ?? error}</span>
                </div>
            )}

            <div className="grid grid-cols-2 lg:grid-cols-5 gap-3">
                {[
                    ['Enabled', status?.enabled ? 'Yes' : 'No', status?.enabled],
                    ['Paused', status?.paused ? 'Yes' : 'No', !status?.paused],
                    ['Bootstrap', status?.bootstrap_passed ? 'Passed' : 'Blocked', status?.bootstrap_passed],
                    ['Session', status?.session_ready ? 'Ready' : 'Blocked', status?.session_ready],
                    ['Actions', status?.action_ready ? 'Ready' : 'Blocked', status?.action_ready],
                ].map(([label, value, ok]) => (
                    <div key={String(label)} className="rounded-lg border border-border/40 bg-card/30 p-4">
                        <p className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">{label}</p>
                        <p className={cn('text-lg font-bold mt-1', ok ? 'text-emerald-400' : 'text-amber-400')}>{value}</p>
                    </div>
                ))}
            </div>

            <div className="rounded-lg border border-border/40 bg-card/30 p-5">
                <div className="flex flex-wrap items-start justify-between gap-4">
                    <div>
                        <h2 className="text-sm font-bold">Runtime Controls</h2>
                        <p className="text-xs text-muted-foreground mt-1">
                            {status ? `${status.profile} / ${status.deployment_mode}` : 'No active desktop autonomy manager'}
                        </p>
                    </div>
                    <div className="flex flex-wrap gap-2">
                        <button
                            disabled={isMutating}
                            onClick={() => runMutation('Bootstrap', thinclaw.bootstrapAutonomy)}
                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs bg-primary/10 text-primary border border-primary/20 hover:bg-primary/20 disabled:opacity-40"
                        >
                            <TerminalSquare className="w-3.5 h-3.5" />
                            Bootstrap
                        </button>
                        <button
                            disabled={!canMutate || isMutating || status?.paused}
                            onClick={() => runMutation('Pause', () => thinclaw.pauseAutonomy('Paused from ThinClaw Desktop'))}
                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs bg-white/[0.03] border border-white/5 hover:bg-white/5 disabled:opacity-40"
                        >
                            <Pause className="w-3.5 h-3.5" />
                            Pause
                        </button>
                        <button
                            disabled={!canMutate || isMutating || !status?.paused}
                            onClick={() => runMutation('Resume', thinclaw.resumeAutonomy)}
                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs bg-white/[0.03] border border-white/5 hover:bg-white/5 disabled:opacity-40"
                        >
                            <Play className="w-3.5 h-3.5" />
                            Resume
                        </button>
                        <button
                            disabled={!canMutate || isMutating || !rollouts?.rollback_target_build_id}
                            onClick={() => runMutation('Rollback', thinclaw.rollbackAutonomy)}
                            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs bg-red-500/10 text-red-300 border border-red-500/20 hover:bg-red-500/20 disabled:opacity-40"
                        >
                            <RotateCcw className="w-3.5 h-3.5" />
                            Rollback
                        </button>
                    </div>
                </div>
                {status?.pause_reason && <p className="text-xs text-muted-foreground mt-3">Pause reason: {status.pause_reason}</p>}
                {status?.blocking_reason && <p className="text-xs text-amber-300 mt-3">Blocking reason: {status.blocking_reason}</p>}
            </div>

            <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
                <div className="rounded-lg border border-border/40 bg-card/30 p-5 space-y-4">
                    <div className="flex items-center gap-2">
                        <ClipboardCheck className="w-4 h-4 text-primary" />
                        <h3 className="text-sm font-bold">Permissions</h3>
                    </div>
                    <JsonBlock value={permissions ?? status?.permission_summary} />
                </div>

                <div className="rounded-lg border border-border/40 bg-card/30 p-5 space-y-4">
                    <div className="flex items-center gap-2">
                        <TerminalSquare className="w-4 h-4 text-primary" />
                        <h3 className="text-sm font-bold">Prerequisites</h3>
                    </div>
                    <JsonBlock value={status?.prerequisite_summary} />
                </div>
            </div>

            <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
                <div className="rounded-lg border border-border/40 bg-card/30 p-5">
                    <div className="flex items-center justify-between gap-3 mb-4">
                        <div className="flex items-center gap-2">
                            <Shield className="w-4 h-4 text-primary" />
                            <h3 className="text-sm font-bold">Rollouts</h3>
                        </div>
                        <span className="text-[10px] text-muted-foreground font-mono">{rollouts?.current_build_id ?? 'no build'}</span>
                    </div>
                    <div className="grid grid-cols-3 gap-3 mb-4">
                        <div className="rounded-lg border border-white/5 bg-black/10 p-3">
                            <p className="text-[10px] uppercase font-bold text-muted-foreground">Failed</p>
                            <p className="text-lg font-bold">{rollouts?.consecutive_failed_promotions ?? 0}</p>
                        </div>
                        <div className="rounded-lg border border-white/5 bg-black/10 p-3">
                            <p className="text-[10px] uppercase font-bold text-muted-foreground">Canaries</p>
                            <p className="text-lg font-bold">{rollouts?.failed_canary_count ?? 0}</p>
                        </div>
                        <div className="rounded-lg border border-white/5 bg-black/10 p-3">
                            <p className="text-[10px] uppercase font-bold text-muted-foreground">Rollback</p>
                            <p className="text-xs font-mono truncate mt-1">{rollouts?.rollback_target_build_id ?? 'none'}</p>
                        </div>
                    </div>
                    <div className="space-y-2 max-h-80 overflow-y-auto">
                        {(rollouts?.recent_builds ?? []).length === 0 ? (
                            <p className="text-xs text-muted-foreground">No rollout builds recorded.</p>
                        ) : rollouts!.recent_builds.map((build) => (
                            <div key={build.build_id} className="rounded-lg border border-white/5 bg-black/10 p-3">
                                <div className="flex items-center justify-between gap-2">
                                    <p className="text-sm font-semibold truncate">{build.title}</p>
                                    <span className={cn('text-[10px] px-2 py-1 rounded-md border', build.promoted ? 'text-emerald-400 bg-emerald-500/10 border-emerald-500/20' : 'text-muted-foreground bg-white/[0.03] border-white/5')}>
                                        {build.promoted ? 'promoted' : 'candidate'}
                                    </span>
                                </div>
                                <p className="text-[10px] text-muted-foreground font-mono mt-1 truncate">{build.build_id}</p>
                            </div>
                        ))}
                    </div>
                </div>

                <div className="rounded-lg border border-border/40 bg-card/30 p-5">
                    <div className="flex items-center gap-2 mb-4">
                        <ClipboardCheck className="w-4 h-4 text-primary" />
                        <h3 className="text-sm font-bold">Checks</h3>
                    </div>
                    <div className="space-y-2 max-h-96 overflow-y-auto">
                        {allChecks.length === 0 ? (
                            <p className="text-xs text-muted-foreground">No checks recorded.</p>
                        ) : allChecks.map((check, index) => <CheckRow key={`${check.name}-${index}`} check={check} />)}
                    </div>
                </div>
            </div>

            <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
                <div className="rounded-lg border border-border/40 bg-card/30 p-5">
                    <div className="flex items-center gap-2 mb-4">
                        <FileSearch className="w-4 h-4 text-primary" />
                        <h3 className="text-sm font-bold">Evidence</h3>
                    </div>
                    <div className="space-y-2 max-h-80 overflow-y-auto">
                        {(evidence?.recent_events ?? []).length === 0 ? (
                            <p className="text-xs text-muted-foreground">No evidence events recorded.</p>
                        ) : evidence!.recent_events.map((event, index) => (
                            <div key={`${event.kind}-${index}`} className="rounded-lg border border-white/5 bg-black/10 p-3">
                                <div className="flex items-center justify-between gap-2">
                                    <p className="text-xs font-semibold">{event.kind}</p>
                                    <p className="text-[10px] text-muted-foreground">{event.timestamp ? new Date(event.timestamp).toLocaleString() : ''}</p>
                                </div>
                                <p className="text-xs text-muted-foreground mt-1">{event.message}</p>
                            </div>
                        ))}
                    </div>
                </div>

                <div className="rounded-lg border border-border/40 bg-card/30 p-5 space-y-4">
                    <h3 className="text-sm font-bold">Latest Bootstrap</h3>
                    <JsonBlock value={lastBootstrap ?? evidence?.latest_bootstrap_report} />
                </div>
            </div>
        </motion.div>
    );
}
