import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import {
    RefreshCw, CheckCircle, XCircle, Clock,
    FileText, Trash2
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';

function OutcomeBadge({ outcome }: { outcome: string }) {
    const styles: Record<string, { icon: any; cls: string }> = {
        success: { icon: CheckCircle, cls: 'text-primary bg-emerald-500/10 border-emerald-500/20' },
        failure: { icon: XCircle, cls: 'text-red-400 bg-red-500/10 border-red-500/20' },
        timeout: { icon: Clock, cls: 'text-muted-foreground bg-amber-500/10 border-amber-500/20' },
    };
    const s = styles[outcome] || styles.failure;
    const Icon = s.icon;
    return (
        <span className={cn("inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-bold uppercase tracking-wider border", s.cls)}>
            <Icon className="w-3 h-3" />
            {outcome}
        </span>
    );
}

interface Props {
    routineKey?: string; // If provided, scoped to one routine
}

export function OpenClawRoutineAudit({ routineKey }: Props) {
    const [entries, setEntries] = useState<openclaw.RoutineAuditEntry[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [filter, setFilter] = useState<'all' | 'success' | 'failure'>('all');
    const [selectedKey, setSelectedKey] = useState(routineKey || '');
    const [cronJobs, setCronJobs] = useState<openclaw.CronJob[]>([]);
    const [confirmClear, setConfirmClear] = useState(false);

    const fetchJobs = useCallback(async () => {
        try {
            const data = await openclaw.getOpenClawCronList();
            setCronJobs(Array.isArray(data) ? data : []);
            if (!selectedKey && Array.isArray(data) && data.length > 0) {
                setSelectedKey(data[0].key);
            }
        } catch (_) { }
    }, [selectedKey]);

    const fetchAudit = useCallback(async () => {
        if (!selectedKey) return;
        setIsLoading(true);
        try {
            const outcome = filter === 'all' ? undefined : filter;
            const data = await openclaw.getRoutineAuditList(selectedKey, 50, outcome as any);
            setEntries(data);
        } catch (e) {
            console.error('Failed to fetch audit entries:', e);
        } finally {
            setIsLoading(false);
        }
    }, [selectedKey, filter]);

    useEffect(() => { fetchJobs(); }, [fetchJobs]);
    useEffect(() => { fetchAudit(); }, [fetchAudit]);

    return (
        <motion.div
            className="flex-1 overflow-y-auto p-8 space-y-6"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
        >
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-orange-500/10 border border-orange-500/20">
                        <FileText className="w-5 h-5 text-muted-foreground" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Routine Audit Log</h1>
                        <p className="text-xs text-muted-foreground">
                            Execution history and outcome tracking for background routines
                        </p>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    {confirmClear ? (
                        <div className="flex items-center gap-1.5 px-2 py-1 rounded-lg bg-red-500/10 border border-red-500/20">
                            <span className="text-[10px] text-red-400 font-medium">Clear {selectedKey ? 'this' : 'all'} history?</span>
                            <button
                                onClick={async () => {
                                    try {
                                        await openclaw.clearRoutineRuns(selectedKey || undefined);
                                        setEntries([]);
                                        fetchAudit();
                                    } catch (e) {
                                        console.error('Failed to clear runs:', e);
                                    }
                                    setConfirmClear(false);
                                }}
                                className="px-2 py-0.5 rounded text-[10px] font-bold text-red-400 bg-red-500/20 hover:bg-red-500/30 transition-all"
                            >
                                Yes
                            </button>
                            <button
                                onClick={() => setConfirmClear(false)}
                                className="px-2 py-0.5 rounded text-[10px] font-bold text-muted-foreground hover:text-foreground transition-all"
                            >
                                No
                            </button>
                        </div>
                    ) : (
                        <button
                            onClick={() => setConfirmClear(true)}
                            disabled={entries.length === 0}
                            className="p-2 rounded-lg text-muted-foreground hover:text-red-400 bg-white/[0.03] hover:bg-red-500/10 border border-white/5 hover:border-red-500/20 transition-all disabled:opacity-30 disabled:pointer-events-none"
                            title="Clear history"
                        >
                            <Trash2 className="w-3.5 h-3.5" />
                        </button>
                    )}
                    <button
                        onClick={fetchAudit}
                        className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                    >
                        <RefreshCw className={cn("w-3.5 h-3.5", isLoading && "animate-spin")} />
                    </button>
                </div>
            </div>

            {/* Controls */}
            <div className="flex items-center gap-3 flex-wrap">
                {/* Routine selector */}
                {cronJobs.length > 0 && (
                    <select
                        value={selectedKey}
                        onChange={(e) => setSelectedKey(e.target.value)}
                        className="px-3 py-1.5 rounded-lg text-xs font-medium bg-white/[0.03] border border-white/5 text-foreground outline-none focus:ring-1 focus:ring-primary/30"
                    >
                        {cronJobs.map(job => (
                            <option key={job.key} value={job.key}>{job.key}</option>
                        ))}
                    </select>
                )}

                {/* Outcome filter */}
                <div className="flex items-center gap-1 p-0.5 rounded-lg bg-white/[0.03] border border-white/5">
                    {(['all', 'success', 'failure'] as const).map(f => (
                        <button
                            key={f}
                            onClick={() => setFilter(f)}
                            className={cn(
                                "px-3 py-1 rounded-md text-[10px] font-bold uppercase tracking-wider transition-all",
                                filter === f ? "bg-primary/15 text-primary" : "text-muted-foreground hover:text-foreground"
                            )}
                        >
                            {f === 'all' ? 'All' : f === 'success' ? '✓ Pass' : '✗ Fail'}
                        </button>
                    ))}
                </div>
            </div>

            {/* Table */}
            <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md overflow-hidden">
                {isLoading ? (
                    <div className="flex items-center justify-center py-16">
                        <RefreshCw className="w-5 h-5 animate-spin text-muted-foreground" />
                    </div>
                ) : entries.length === 0 ? (
                    <div className="text-center py-16 space-y-2">
                        <Clock className="w-8 h-8 text-muted-foreground/30 mx-auto" />
                        <p className="text-sm text-muted-foreground">No audit entries yet</p>
                        <p className="text-xs text-muted-foreground/60">
                            Entries appear after routines execute
                        </p>
                    </div>
                ) : (
                    <table className="w-full text-xs">
                        <thead>
                            <tr className="border-b border-white/5">
                                <th className="text-left px-4 py-3 text-[10px] text-muted-foreground font-bold uppercase tracking-widest">Routine</th>
                                <th className="text-left px-4 py-3 text-[10px] text-muted-foreground font-bold uppercase tracking-widest">Started</th>
                                <th className="text-left px-4 py-3 text-[10px] text-muted-foreground font-bold uppercase tracking-widest">Duration</th>
                                <th className="text-left px-4 py-3 text-[10px] text-muted-foreground font-bold uppercase tracking-widest">Outcome</th>
                                <th className="text-left px-4 py-3 text-[10px] text-muted-foreground font-bold uppercase tracking-widest">Error</th>
                            </tr>
                        </thead>
                        <tbody>
                            {entries.map((entry, i) => (
                                <motion.tr
                                    key={`${entry.routine_key}-${entry.started_at}-${i}`}
                                    initial={{ opacity: 0, x: -8 }}
                                    animate={{ opacity: 1, x: 0 }}
                                    transition={{ delay: i * 0.02 }}
                                    className="border-b border-white/[0.03] hover:bg-white/[0.02]"
                                >
                                    <td className="px-4 py-3 font-mono text-muted-foreground">{entry.routine_key}</td>
                                    <td className="px-4 py-3 text-muted-foreground whitespace-nowrap">{formatTime(entry.started_at)}</td>
                                    <td className="px-4 py-3 text-muted-foreground tabular-nums">
                                        {entry.duration_ms != null ? `${entry.duration_ms}ms` : '—'}
                                    </td>
                                    <td className="px-4 py-3"><OutcomeBadge outcome={entry.outcome} /></td>
                                    <td className="px-4 py-3 text-red-400/70 max-w-[200px] truncate">{entry.error || '—'}</td>
                                </motion.tr>
                            ))}
                        </tbody>
                    </table>
                )}
            </div>
        </motion.div>
    );
}

function formatTime(iso: string): string {
    try {
        const d = new Date(iso);
        return d.toLocaleString('en-US', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit' });
    } catch {
        return iso;
    }
}
