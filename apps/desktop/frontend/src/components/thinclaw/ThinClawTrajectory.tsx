import { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import { Activity, RefreshCw, FileText, Layers, CheckCircle2, XCircle, Clock, Download } from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';

function StatCard({ icon: Icon, label, value, sub, color }: {
    icon: any; label: string; value: string; sub?: string; color: string;
}) {
    return (
        <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-5">
            <div className="flex items-center gap-2 mb-3">
                <Icon className={cn('w-4 h-4', color)} />
                <span className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">{label}</span>
            </div>
            <p className={cn('text-3xl font-bold tabular-nums', color)}>{value}</p>
            {sub && <p className="text-[10px] text-muted-foreground mt-1">{sub}</p>}
        </div>
    );
}

/** Defensive accessor — trajectory records are raw JSON whose exact shape can evolve. */
function field(record: thinclaw.ThinClawJson, ...keys: string[]): string {
    const obj = (record ?? {}) as Record<string, unknown>;
    for (const k of keys) {
        const v = obj[k];
        if (typeof v === 'string' && v.trim()) return v;
        if (typeof v === 'number') return String(v);
    }
    return '';
}

export function ThinClawTrajectory() {
    const [stats, setStats] = useState<thinclaw.TrajectoryStats | null>(null);
    const [records, setRecords] = useState<thinclaw.ThinClawJson[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [exporting, setExporting] = useState<thinclaw.TrajectoryExportFormat | null>(null);

    const fetchData = useCallback(async () => {
        try {
            const [s, r] = await Promise.all([
                thinclaw.getTrajectoryStats(),
                thinclaw.getTrajectoryRecords(50),
            ]);
            setStats(s);
            setRecords(Array.isArray(r) ? r.slice().reverse() : []);
        } catch (e) {
            console.error('Failed to fetch trajectory archive:', e);
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        fetchData();
    }, [fetchData]);

    const handleExport = async (format: thinclaw.TrajectoryExportFormat) => {
        setExporting(format);
        try {
            const result = await thinclaw.exportTrajectory(format);
            if (result.exported_record_count === 0) {
                toast.error(`No eligible ${format.toUpperCase()} examples were found`);
                return;
            }

            const blob = new Blob([result.payload], { type: 'application/x-ndjson;charset=utf-8' });
            const url = URL.createObjectURL(blob);
            const link = document.createElement('a');
            const stamp = new Date().toISOString().replace(/[:.]/g, '-');
            link.href = url;
            link.download = `thinclaw-trajectory-${format}-${stamp}.jsonl`;
            link.click();
            URL.revokeObjectURL(url);
            toast.success(`Exported ${result.exported_record_count.toLocaleString()} ${format.toUpperCase()} examples`);
        } catch (error) {
            toast.error(`Trajectory export failed: ${String(error)}`);
        } finally {
            setExporting(null);
        }
    };

    if (isLoading) {
        return (
            <div className="flex-1 flex items-center justify-center">
                <RefreshCw className="w-5 h-5 animate-spin text-muted-foreground" />
            </div>
        );
    }

    const success = stats?.success_count ?? 0;
    const failure = stats?.failure_count ?? 0;
    const neutral = stats?.neutral_count ?? 0;
    const scored = success + failure + neutral;
    const successRate = scored > 0 ? success / scored : 0;
    const span = stats?.first_seen && stats?.last_seen
        ? `${new Date(stats.first_seen).toLocaleDateString()} → ${new Date(stats.last_seen).toLocaleDateString()}`
        : 'no turns recorded yet';

    return (
        <motion.div
            className="flex-1 overflow-y-auto p-8 space-y-8"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
        >
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-cyan-500/10 border border-cyan-500/20">
                        <Activity className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Trajectory Archive</h1>
                        <p className="text-xs text-muted-foreground">
                            Per-turn records used for learning feedback and SFT/DPO export · {span}
                        </p>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    {(['sft', 'dpo'] as const).map(format => (
                        <button
                            key={format}
                            onClick={() => handleExport(format)}
                            disabled={exporting !== null || (stats?.record_count ?? 0) === 0}
                            className="flex items-center gap-1.5 px-3 py-2 rounded-lg text-[11px] font-semibold uppercase tracking-wide text-primary bg-primary/10 hover:bg-primary/15 border border-primary/20 transition-all disabled:opacity-40 disabled:cursor-not-allowed"
                        >
                            {exporting === format
                                ? <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                                : <Download className="w-3.5 h-3.5" />}
                            Export {format.toUpperCase()}
                        </button>
                    ))}
                    <button
                        aria-label="Refresh trajectory archive"
                        onClick={fetchData}
                        className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/3 hover:bg-white/5 border border-white/5 transition-all"
                    >
                        <RefreshCw className="w-3.5 h-3.5" />
                    </button>
                </div>
            </div>

            {/* Stat cards */}
            <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
                <StatCard icon={FileText} label="Turns" value={(stats?.record_count ?? 0).toLocaleString()} sub={`${(stats?.file_count ?? 0).toLocaleString()} files`} color="text-primary" />
                <StatCard icon={Layers} label="Sessions" value={(stats?.session_count ?? 0).toLocaleString()} sub="distinct threads" color="text-blue-400" />
                <StatCard icon={CheckCircle2} label="Positive" value={success.toLocaleString()} sub={`${(successRate * 100).toFixed(0)}% of scored`} color="text-emerald-400" />
                <StatCard icon={XCircle} label="Negative" value={failure.toLocaleString()} sub={`${neutral.toLocaleString()} neutral`} color={failure > 0 ? 'text-red-400' : 'text-muted-foreground'} />
            </div>

            {/* Outcome breakdown */}
            <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-6">
                <h3 className="text-sm font-bold text-muted-foreground mb-4">Outcome Distribution</h3>
                <div className="h-6 bg-white/3 rounded-full overflow-hidden border border-white/5 flex">
                    <motion.div initial={{ width: 0 }} animate={{ width: `${scored ? (success / scored) * 100 : 0}%` }} transition={{ duration: 0.7, ease: 'easeOut' }} className="h-full bg-emerald-500/70" />
                    <motion.div initial={{ width: 0 }} animate={{ width: `${scored ? (neutral / scored) * 100 : 0}%` }} transition={{ duration: 0.7, ease: 'easeOut' }} className="h-full bg-white/15" />
                    <motion.div initial={{ width: 0 }} animate={{ width: `${scored ? (failure / scored) * 100 : 0}%` }} transition={{ duration: 0.7, ease: 'easeOut' }} className="h-full bg-red-500/70" />
                </div>
                <div className="flex gap-4 mt-2 text-[9px] text-muted-foreground">
                    <span className="flex items-center gap-1"><span className="w-2 h-2 rounded-full bg-emerald-500/70" /> positive</span>
                    <span className="flex items-center gap-1"><span className="w-2 h-2 rounded-full bg-white/15" /> neutral</span>
                    <span className="flex items-center gap-1"><span className="w-2 h-2 rounded-full bg-red-500/70" /> negative</span>
                </div>
            </div>

            {/* Recent records */}
            <div className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md p-6">
                <h3 className="text-sm font-bold text-muted-foreground mb-4">Recent Turns</h3>
                {records.length === 0 ? (
                    <p className="text-xs text-muted-foreground">No trajectory records yet.</p>
                ) : (
                    <div className="space-y-2">
                        {records.map((r, i) => {
                            const user = field(r, 'user_message', 'user_input', 'input');
                            const ts = field(r, 'timestamp', 'created_at', 'ts');
                            const model = field(r, 'model', 'llm_model');
                            const score = field(r, 'outcome_score', 'score', 'heuristic_score');
                            return (
                                <div key={i} className="rounded-lg border border-white/5 bg-white/2 px-3 py-2">
                                    <div className="flex items-center justify-between gap-2">
                                        <p className="text-xs truncate text-foreground/90">{user || '(no user message)'}</p>
                                        {score && <span className="text-[10px] tabular-nums text-muted-foreground shrink-0">{Number(score).toFixed(2)}</span>}
                                    </div>
                                    <div className="flex items-center gap-3 mt-1 text-[9px] text-muted-foreground">
                                        {ts && <span className="flex items-center gap-1"><Clock className="w-2.5 h-2.5" />{new Date(ts).toLocaleString()}</span>}
                                        {model && <span>{model}</span>}
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                )}
            </div>
        </motion.div>
    );
}
