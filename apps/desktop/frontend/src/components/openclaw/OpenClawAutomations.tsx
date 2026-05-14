import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { listen } from '@tauri-apps/api/event';
import {
    Timer,
    Play,
    History as HistoryIcon,
    Plus,
    RefreshCw,
    Clock,
    AlertCircle,
    CheckCircle2,
    Calendar,
    X,
    Zap,
    Search,
    ArrowRight,
    Terminal,
    FileText,
    Heart,
    Activity,
    Gauge,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

interface JobCardProps {
    job: openclaw.CronJob;
    onRun: (key: string) => void;
    onViewHistory: (key: string) => void;
    onDelete: (key: string, name: string) => void;
    onRefresh?: () => void;
}

function JobCard({ job, onRun, onViewHistory, onDelete, onRefresh }: JobCardProps) {
    const [confirmingDelete, setConfirmingDelete] = useState(false);
    const [updatingInterval, setUpdatingInterval] = useState(false);

    // Parse current interval from cron schedule like "*/30 * * * *" or "0 */30 * * * * *"
    const parseIntervalMinutes = (): number => {
        const schedule = job.schedule || '';
        // Match 7-field: "0 */N * * * * *" or 5-field embedded: "*/N * * * *"
        const match7 = schedule.match(/^0\s+\*\/(\d+)\s+\*\s+\*\s+\*\s+\*\s+\*$/);
        if (match7) return parseInt(match7[1], 10);
        const match5 = schedule.match(/^\*\/(\d+)\s+\*\s+\*\s+\*\s+\*$/);
        if (match5) return parseInt(match5[1], 10);
        return 30; // default
    };

    const currentInterval = parseIntervalMinutes();

    const INTERVAL_PRESETS = [
        { label: '5m', minutes: 5 },
        { label: '10m', minutes: 10 },
        { label: '15m', minutes: 15 },
        { label: '30m', minutes: 30 },
        { label: '1h', minutes: 60 },
        { label: '2h', minutes: 120 },
    ];

    const handleSetInterval = async (minutes: number) => {
        if (minutes === currentInterval) return;
        setUpdatingInterval(true);
        try {
            await openclaw.setHeartbeatInterval(minutes);
            toast.success(`Heartbeat interval set to ${minutes < 60 ? `${minutes} min` : `${minutes / 60}h`}`);
            onRefresh?.();
        } catch (e) {
            toast.error(`Failed to update interval: ${String(e)}`);
        } finally {
            setUpdatingInterval(false);
        }
    };
    return (
        <div className="p-5 rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm border-border/40 group relative overflow-hidden">
            <div className="flex items-start justify-between mb-4">
                <div className="flex items-center gap-3">
                    <div className={cn("p-2 rounded-lg", job.action_type === 'heartbeat' ? 'bg-rose-500/10' : 'bg-primary/10')}>
                        {job.action_type === 'heartbeat' ? (
                            <Heart className="w-5 h-5 text-rose-400 animate-pulse" />
                        ) : (
                            <Timer className="w-5 h-5 text-primary" />
                        )}
                    </div>
                    <div>
                        <div className="flex items-center gap-2">
                            <h3 className="font-semibold">{job.name === '__heartbeat__' ? 'Heartbeat' : (job.name ?? job.key)}</h3>
                            {job.action_type === 'heartbeat' && (
                                <span className="flex items-center gap-1 px-2 py-0.5 rounded-full bg-rose-500/10 border border-rose-500/20 text-[9px] font-bold uppercase tracking-wider text-rose-400">
                                    <Heart className="w-2.5 h-2.5 fill-current" />
                                    Heartbeat
                                </span>
                            )}
                            {job.trigger_type === 'system_event' && (
                                <span className="flex items-center gap-1 px-2 py-0.5 rounded-full bg-violet-500/10 border border-violet-500/20 text-[9px] font-bold uppercase tracking-wider text-violet-400">
                                    <Activity className="w-2.5 h-2.5" />
                                    Event
                                </span>
                            )}
                        </div>
                        <p className="text-xs text-muted-foreground">{job.description}</p>
                    </div>
                </div>
                <div className="flex items-center gap-1 opacity-100 group-hover:opacity-100 transition-opacity">
                    <button
                        onClick={() => onViewHistory(job.key)}
                        className="p-1.5 rounded-md hover:bg-white/5 text-muted-foreground transition-colors"
                        title="View History"
                    >
                        <HistoryIcon className="w-4 h-4" />
                    </button>
                    <button
                        onClick={() => onRun(job.key)}
                        className="p-1.5 rounded-md hover:bg-white/5 text-primary transition-colors"
                        title="Run Now"
                    >
                        <Play className="w-4 h-4 fill-current" />
                    </button>
                    {/* Only show delete button for non-system routines */}
                    {job.name !== '__heartbeat__' && (
                        <button
                            onClick={() => setConfirmingDelete(true)}
                            className="p-1.5 rounded-md hover:bg-red-500/10 text-muted-foreground hover:text-red-400 transition-colors"
                            title="Delete Routine"
                        >
                            <X className="w-4 h-4" />
                        </button>
                    )}
                </div>
            </div>

            <div className="grid grid-cols-2 gap-4 mt-6">
                <div className="space-y-1">
                    <p className="text-[10px] uppercase font-bold text-muted-foreground tracking-widest flex items-center gap-1.5">
                        <Calendar className="w-3 h-3" />
                        Schedule
                    </p>
                    <p className="text-sm font-mono text-primary/80">{job.schedule}</p>
                </div>
                <div className="space-y-1 text-right border-l border-white/5 pl-4">
                    <p className="text-[10px] uppercase font-bold text-muted-foreground tracking-widest flex items-center gap-1.5 justify-end">
                        <Clock className="w-3 h-3" />
                        Next Run
                    </p>
                    <p className="text-xs text-muted-foreground truncate">{job.nextRun || 'Calculating...'}</p>
                </div>
            </div>

            {/* ── Heartbeat interval control ───────────────────────────── */}
            {job.action_type === 'heartbeat' && (
                <div className="mt-4 pt-4 border-t border-white/5 space-y-2">
                    <p className="text-[10px] uppercase font-bold text-muted-foreground tracking-widest flex items-center gap-1.5">
                        <Gauge className="w-3 h-3" />
                        Interval
                    </p>
                    <div className="flex flex-wrap gap-1.5">
                        {INTERVAL_PRESETS.map(p => (
                            <button
                                key={p.minutes}
                                onClick={() => handleSetInterval(p.minutes)}
                                disabled={updatingInterval}
                                className={cn(
                                    'px-2.5 py-1 rounded-lg text-[10px] font-semibold border transition-all',
                                    currentInterval === p.minutes
                                        ? 'bg-rose-500/15 text-rose-400 border-rose-500/30 shadow-sm shadow-rose-500/10'
                                        : 'bg-white/[0.03] text-muted-foreground hover:bg-white/5 border-border/30 hover:border-rose-500/20',
                                    updatingInterval && 'opacity-50 cursor-not-allowed'
                                )}
                            >
                                {p.label}
                            </button>
                        ))}
                    </div>
                </div>
            )}

            <div className="mt-4 pt-4 border-t border-white/5 flex items-center justify-between">
                <div className="flex items-center gap-2">
                    {job.lastStatus === 'ok' ? (
                        <CheckCircle2 className="w-3.5 h-3.5 text-green-500" />
                    ) : job.lastStatus === 'error' ? (
                        <AlertCircle className="w-3.5 h-3.5 text-red-500" />
                    ) : (
                        <CircleIcon className="w-3.5 h-3.5 text-muted-foreground/30" />
                    )}
                    <span className="text-[10px] text-muted-foreground uppercase font-bold tracking-tight">
                        Last Exit: {job.lastStatus || 'Never'}
                    </span>
                </div>
                <span className="text-[10px] text-muted-foreground font-mono">
                    {job.lastRun ? `Ran ${job.lastRun}` : 'No history'}
                </span>
            </div>

            {/* Inline delete confirmation — replaces broken window.confirm() in Tauri WebView */}
            <AnimatePresence>
                {confirmingDelete && (
                    <motion.div
                        initial={{ opacity: 0 }}
                        animate={{ opacity: 1 }}
                        exit={{ opacity: 0 }}
                        className="absolute inset-0 bg-red-950/95 backdrop-blur-sm rounded-2xl flex flex-col items-center justify-center gap-3 z-10 p-4"
                    >
                        <p className="text-sm font-semibold text-red-200 text-center">
                            Delete <span className="text-white font-bold">"{job.name ?? job.key}"</span>?
                        </p>
                        <p className="text-[10px] text-red-400/70">This cannot be undone.</p>
                        <div className="flex gap-2 mt-1">
                            <button
                                onClick={() => setConfirmingDelete(false)}
                                className="px-4 py-1.5 rounded-lg text-xs font-medium bg-white/10 text-white hover:bg-white/20 transition-all border border-border/40"
                            >
                                Cancel
                            </button>
                            <button
                                onClick={() => { setConfirmingDelete(false); onDelete(job.key, job.name ?? job.key); }}
                                className="px-4 py-1.5 rounded-lg text-xs font-semibold bg-red-500 text-white hover:bg-red-400 transition-all shadow-lg shadow-red-500/30"
                            >
                                Delete
                            </button>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
}

// Minimal circle for status
function CircleIcon({ className }: { className?: string }) {
    return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" className={className}>
            <circle cx="12" cy="12" r="10" />
        </svg>
    );
}

// ── Create Job Modal ─────────────────────────────────────────────────

interface CreateJobModalProps {
    onClose: () => void;
    onCreated: () => void;
}

const SCHEDULE_PRESETS = [
    { label: 'Every minute', value: '0 * * * * * *' },
    { label: 'Every 5 min', value: '0 */5 * * * * *' },
    { label: 'Every 15 min', value: '0 */15 * * * * *' },
    { label: 'Every hour', value: '0 0 * * * * *' },
    { label: 'Daily at 9am', value: '0 0 9 * * * *' },
    { label: 'Daily midnight', value: '0 0 0 * * * *' },
    { label: 'Weekly Mon', value: '0 0 9 * * 1 *' },
];

function CreateJobModal({ onClose, onCreated }: CreateJobModalProps) {
    const [name, setName] = useState('');
    const [description, setDescription] = useState('');
    const [schedule, setSchedule] = useState('0 0 * * * * *');
    const [task, setTask] = useState('');
    const [isSubmitting, setIsSubmitting] = useState(false);
    const [lintResult, setLintResult] = useState<openclaw.CronLintResult | null>(null);
    const [lintError, setLintError] = useState<string | null>(null);
    const [isLinting, setIsLinting] = useState(false);

    // Auto-lint when schedule changes
    useEffect(() => {
        if (!schedule.trim()) { setLintResult(null); setLintError(null); return; }
        const timer = setTimeout(async () => {
            setIsLinting(true);
            setLintError(null);
            try {
                const r = await openclaw.lintCronExpression(schedule.trim());
                setLintResult(r);
            } catch (e) {
                setLintError(String(e));
                setLintResult(null);
            } finally {
                setIsLinting(false);
            }
        }, 400);
        return () => clearTimeout(timer);
    }, [schedule]);

    const handleSubmit = async (e: React.FormEvent) => {
        e.preventDefault();
        if (!name.trim() || !schedule.trim() || !task.trim()) {
            toast.error('Name, schedule, and task are required');
            return;
        }
        if (lintError) {
            toast.error('Fix the cron expression before saving');
            return;
        }
        setIsSubmitting(true);
        try {
            await openclaw.createRoutine(
                name.trim(),
                description.trim(),
                schedule.trim(),
                task.trim(),
            );
            toast.success(`Routine "${name}" created successfully`);
            onCreated();
            onClose();
        } catch (e) {
            toast.error(`Failed to create routine: ${String(e)}`);
        } finally {
            setIsSubmitting(false);
        }
    };

    const inputCls = 'w-full h-9 rounded-lg border border-border/40 bg-white/[0.03] px-3 text-sm text-zinc-200 placeholder:text-muted-foreground/40 focus:outline-none focus:ring-2 focus:ring-primary/30 focus:border-primary/40 transition-all';

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
            <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                className="absolute inset-0 bg-black/60 backdrop-blur-sm"
                onClick={onClose}
            />
            <motion.div
                initial={{ scale: 0.95, opacity: 0, y: 10 }}
                animate={{ scale: 1, opacity: 1, y: 0 }}
                exit={{ scale: 0.95, opacity: 0, y: 10 }}
                transition={{ type: 'spring', stiffness: 400, damping: 30 }}
                className="relative w-full max-w-lg bg-zinc-950/95 backdrop-blur-xl border border-border/40 rounded-2xl shadow-2xl flex flex-col overflow-hidden"
            >
                {/* Header */}
                <div className="flex items-center justify-between px-6 py-4 border-b border-border/40 bg-white/[0.02]">
                    <div className="flex items-center gap-3">
                        <div className="p-2 bg-primary/10 rounded-lg">
                            <Timer className="w-4 h-4 text-primary" />
                        </div>
                        <div>
                            <h2 className="text-base font-bold">Create Scheduled Job</h2>
                            <p className="text-[11px] text-muted-foreground">Stored in ThinClaw RoutineStore — survives restarts</p>
                        </div>
                    </div>
                    <button onClick={onClose} className="p-1.5 hover:bg-white/10 rounded-lg transition-colors text-muted-foreground hover:text-white">
                        <X className="w-4 h-4" />
                    </button>
                </div>

                {/* Form */}
                <form onSubmit={handleSubmit} className="flex-1 overflow-y-auto">
                    <div className="px-6 py-5 space-y-5">
                        {/* Name */}
                        <div className="space-y-1.5">
                            <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70">
                                Job Name <span className="text-red-400">*</span>
                            </label>
                            <input
                                type="text"
                                value={name}
                                onChange={e => setName(e.target.value)}
                                placeholder="e.g. daily-cleanup"
                                className={inputCls}
                                required
                            />
                        </div>

                        {/* Description */}
                        <div className="space-y-1.5">
                            <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70">
                                Description
                            </label>
                            <input
                                type="text"
                                value={description}
                                onChange={e => setDescription(e.target.value)}
                                placeholder="What does this job do?"
                                className={inputCls}
                            />
                        </div>

                        {/* Schedule presets */}
                        <div className="space-y-2">
                            <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70">
                                Schedule Presets
                            </label>
                            <div className="flex flex-wrap gap-1.5">
                                {SCHEDULE_PRESETS.map(p => (
                                    <button
                                        key={p.value}
                                        type="button"
                                        onClick={() => setSchedule(p.value)}
                                        className={cn(
                                            'px-2.5 py-1 rounded-lg text-[10px] font-medium border transition-all',
                                            schedule === p.value
                                                ? 'bg-primary/15 text-primary border-primary/30'
                                                : 'bg-white/[0.03] text-muted-foreground hover:bg-white/5 border-border/30',
                                        )}
                                    >
                                        {p.label}
                                    </button>
                                ))}
                            </div>
                        </div>

                        {/* Cron expression + lint */}
                        <div className="space-y-1.5">
                            <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70">
                                Cron Expression <span className="text-red-400">*</span>
                            </label>
                            <div className="relative">
                                <input
                                    type="text"
                                    value={schedule}
                                    onChange={e => setSchedule(e.target.value)}
                                    placeholder="0 0 * * * * *"
                                    className={cn(inputCls, 'font-mono pr-8')}
                                    required
                                />
                                {isLinting && (
                                    <RefreshCw className="absolute right-2.5 top-2.5 w-3.5 h-3.5 text-muted-foreground animate-spin" />
                                )}
                                {!isLinting && lintResult && (
                                    <CheckCircle2 className="absolute right-2.5 top-2.5 w-3.5 h-3.5 text-green-500" />
                                )}
                                {!isLinting && lintError && (
                                    <AlertCircle className="absolute right-2.5 top-2.5 w-3.5 h-3.5 text-red-400" />
                                )}
                            </div>
                            <p className="text-[10px] text-muted-foreground/50">
                                Format: sec min hour dom month dow year
                            </p>
                            <AnimatePresence>
                                {lintError && (
                                    <motion.p
                                        initial={{ opacity: 0, y: -4 }}
                                        animate={{ opacity: 1, y: 0 }}
                                        exit={{ opacity: 0 }}
                                        className="text-[10px] text-red-400 font-mono"
                                    >
                                        {lintError}
                                    </motion.p>
                                )}
                                {lintResult && (
                                    <motion.div
                                        initial={{ opacity: 0, y: -4 }}
                                        animate={{ opacity: 1, y: 0 }}
                                        exit={{ opacity: 0 }}
                                        className="flex items-center gap-2 text-[10px] text-green-400"
                                    >
                                        <span>Next: {new Date(lintResult.next_fire_times[0]).toLocaleString()}</span>
                                    </motion.div>
                                )}
                            </AnimatePresence>
                        </div>

                        {/* Task prompt */}
                        <div className="space-y-1.5">
                            <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70 flex items-center gap-1.5">
                                <Terminal className="w-3 h-3" />
                                Agent Task Prompt <span className="text-red-400">*</span>
                            </label>
                            <textarea
                                value={task}
                                onChange={e => setTask(e.target.value)}
                                placeholder="Describe what the agent should do when this job fires. E.g: 'Summarize yesterday's logs and write a report to /workspace/reports/daily.md'"
                                rows={4}
                                className={cn(inputCls, 'h-auto py-2 resize-y text-xs leading-relaxed')}
                                required
                            />
                            <p className="text-[10px] text-muted-foreground/50 flex items-center gap-1.5">
                                <FileText className="w-3 h-3" />
                                This prompt is sent to the agent automatically when the schedule fires.
                            </p>
                        </div>
                    </div>

                    {/* Footer */}
                    <div className="px-6 py-4 border-t border-border/40 bg-white/[0.01] flex items-center justify-end gap-3">
                        <button
                            type="button"
                            onClick={onClose}
                            className="px-4 py-2 rounded-lg text-sm text-muted-foreground hover:text-white hover:bg-white/5 transition-all border border-border/40"
                        >
                            Cancel
                        </button>
                        <button
                            type="submit"
                            disabled={isSubmitting || !name.trim() || !schedule.trim() || !task.trim() || !!lintError}
                            className={cn(
                                'flex items-center gap-2 px-5 py-2 rounded-lg text-sm font-semibold border transition-all shadow-lg shadow-primary/20',
                                'bg-primary/20 text-primary border-primary/30 hover:bg-primary/30',
                                (isSubmitting || !name.trim() || !schedule.trim() || !task.trim() || !!lintError) &&
                                'opacity-40 cursor-not-allowed',
                            )}
                        >
                            {isSubmitting ? (
                                <><RefreshCw className="w-3.5 h-3.5 animate-spin" /> Creating…</>
                            ) : (
                                <><Plus className="w-3.5 h-3.5" /> Create Job</>
                            )}
                        </button>
                    </div>
                </form>
            </motion.div>
        </div>
    );
}

// ── Main Component ────────────────────────────────────────────────────

export function OpenClawAutomations() {
    const [jobs, setJobs] = useState<openclaw.CronJob[]>([]);
    const [historyJob, setHistoryJob] = useState<string | null>(null);
    const [history, setHistory] = useState<any[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [showCreateModal, setShowCreateModal] = useState(false);

    // Cron lint state
    const [cronExpr, setCronExpr] = useState('');
    const [lintResult, setLintResult] = useState<openclaw.CronLintResult | null>(null);
    const [lintError, setLintError] = useState<string | null>(null);
    const [isLinting, setIsLinting] = useState(false);

    const fetchData = async () => {
        try {
            const data = await openclaw.getOpenClawCronList();
            setJobs(Array.isArray(data) ? data : []);
        } catch (e) {
            console.error('Failed to fetch cron jobs:', e);
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchData();
        const interval = setInterval(fetchData, 30000);
        return () => clearInterval(interval);
    }, []);

    // Listen for routine lifecycle events from backend SSE forwarder
    useEffect(() => {
        const unlistenPromise = listen<any>('openclaw-event', (event) => {
            const payload = event.payload;
            if (payload?.kind === 'RoutineLifecycle') {
                const { routine_name, event: evType, result_summary } = payload;
                const snippet = result_summary ? `: ${String(result_summary).slice(0, 80)}` : '';
                if (evType === 'started') {
                    toast.info(`⏱ "${routine_name}" started`, { duration: 4000 });
                } else if (evType === 'dispatched') {
                    // full_job was queued — worker is running, real result comes later
                    toast.info(`🔄 "${routine_name}" queued — worker executing`, { duration: 5000 });
                } else if (evType === 'completed') {
                    toast.success(`✅ "${routine_name}" completed${snippet}`, { duration: 6000 });
                    fetchData();
                } else if (evType === 'failed') {
                    toast.error(`❌ "${routine_name}" failed${snippet}`, { duration: 8000 });
                    fetchData();
                }
            }
        });
        return () => { unlistenPromise.then((fn) => fn()).catch(() => { }); };
    }, [fetchData]);

    const handleRun = async (key: string) => {
        try {
            toast.promise(openclaw.runOpenClawCron(key), {
                loading: `Triggering routine...`,
                success: `Routine triggered — watch output below`,
                error: (err) => `Failed to run: ${err}`
            });
        } catch (_e) { }
    };

    const handleDelete = async (key: string, name: string) => {
        try {
            await openclaw.deleteRoutine(key);
            setJobs(prev => prev.filter(j => j.key !== key));
            toast.success(`Routine "${name}" deleted`);
        } catch (e) {
            toast.error(`Failed to delete: ${String(e)}`);
        }
    };

    const handleViewHistory = async (key: string) => {
        setHistoryJob(key);
        setHistory([]);
        try {
            const data = await openclaw.getRoutineAuditList(key, 10);
            setHistory(Array.isArray(data) ? data : []);
        } catch (_e) {
            toast.error(`Failed to fetch history for ${key}`);
        }
    };

    const handleLintCron = useCallback(async () => {
        if (!cronExpr.trim()) return;
        setIsLinting(true);
        setLintError(null);
        setLintResult(null);
        try {
            const result = await openclaw.lintCronExpression(cronExpr.trim());
            setLintResult(result);
        } catch (e) {
            setLintError(String(e));
        } finally {
            setIsLinting(false);
        }
    }, [cronExpr]);

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 p-8 space-y-8 max-w-6xl mx-auto"
        >
            <div className="flex items-center justify-between">
                <div>
                    <h1 className="text-3xl font-bold tracking-tight">Automations</h1>
                    <p className="text-muted-foreground mt-1">Managed cron jobs and scheduled agent tasks.</p>
                </div>
                <div className="flex items-center gap-2">
                    <button
                        onClick={() => setShowCreateModal(true)}
                        className="flex items-center gap-2 px-4 py-2 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:opacity-90 transition-all shadow-lg shadow-primary/20"
                    >
                        <Plus className="w-4 h-4" />
                        Create Job
                    </button>
                    <button
                        onClick={() => {
                            setIsLoading(true);
                            fetchData();
                        }}
                        className="p-2.5 rounded-lg bg-card border border-border/40 hover:bg-white/5 transition-colors"
                    >
                        <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                    </button>
                </div>
            </div>

            {/* ── Cron Expression Validator ──────────────────────────────── */}
            <div className="p-6 rounded-2xl border bg-card/30 backdrop-blur-md border-border/40 space-y-4">
                <div className="flex items-center gap-2">
                    <Search className="w-4 h-4 text-primary" />
                    <h2 className="text-sm font-bold uppercase tracking-wider text-muted-foreground">Cron Expression Validator</h2>
                </div>
                <div className="flex gap-3">
                    <input
                        type="text"
                        value={cronExpr}
                        onChange={e => setCronExpr(e.target.value)}
                        onKeyDown={e => e.key === 'Enter' && handleLintCron()}
                        placeholder="0 */5 * * * * *  (sec min hour dom month dow year)"
                        className="flex-1 px-4 py-2.5 rounded-xl bg-white/[0.03] border border-border/40 text-sm font-mono placeholder:text-muted-foreground/40 focus:outline-none focus:ring-2 focus:ring-primary/30 focus:border-primary/40 transition-all"
                    />
                    <button
                        onClick={handleLintCron}
                        disabled={!cronExpr.trim() || isLinting}
                        className="px-5 py-2.5 rounded-xl bg-primary text-primary-foreground text-sm font-medium hover:opacity-90 transition-all disabled:opacity-40 flex items-center gap-2 shadow-lg shadow-primary/20"
                    >
                        {isLinting ? <RefreshCw className="w-3.5 h-3.5 animate-spin" /> : <ArrowRight className="w-3.5 h-3.5" />}
                        Validate
                    </button>
                </div>

                <AnimatePresence mode="wait">
                    {lintError && (
                        <motion.div
                            key="error"
                            initial={{ opacity: 0, y: -5 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={{ opacity: 0, y: -5 }}
                            className="p-4 rounded-xl bg-red-500/5 border border-red-500/20 flex items-start gap-3"
                        >
                            <AlertCircle className="w-4 h-4 text-red-400 mt-0.5 flex-shrink-0" />
                            <div>
                                <p className="text-sm font-medium text-red-400">Invalid Expression</p>
                                <p className="text-xs text-red-400/70 font-mono mt-1">{lintError}</p>
                            </div>
                        </motion.div>
                    )}
                    {lintResult && (
                        <motion.div
                            key="result"
                            initial={{ opacity: 0, y: -5 }}
                            animate={{ opacity: 1, y: 0 }}
                            exit={{ opacity: 0, y: -5 }}
                            className="p-4 rounded-xl bg-green-500/5 border border-green-500/20 space-y-3"
                        >
                            <div className="flex items-center gap-2">
                                <CheckCircle2 className="w-4 h-4 text-green-500" />
                                <span className="text-sm font-medium text-green-500">Valid Expression</span>
                                <span className="text-xs font-mono text-muted-foreground ml-auto">{lintResult.expression}</span>
                            </div>
                            <div className="space-y-2">
                                <p className="text-[10px] uppercase font-bold text-muted-foreground tracking-widest">Next 5 Fire Times</p>
                                <div className="space-y-1.5">
                                    {lintResult.next_fire_times.map((t, i) => (
                                        <div key={i} className="flex items-center gap-3 text-xs">
                                            <span className="w-5 h-5 rounded-full bg-primary/10 text-primary flex items-center justify-center text-[10px] font-bold">
                                                {i + 1}
                                            </span>
                                            <Clock className="w-3 h-3 text-muted-foreground/50" />
                                            <span className="font-mono text-muted-foreground">
                                                {new Date(t).toLocaleString()}
                                            </span>
                                        </div>
                                    ))}
                                </div>
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
                {isLoading && jobs.length === 0 ? (
                    [1, 2, 3, 4].map(i => (
                        <div key={i} className="h-48 rounded-2xl border border-white/5 bg-white/[0.02] animate-pulse" />
                    ))
                ) : jobs.length > 0 ? (
                    jobs.map(job => (
                        <JobCard
                            key={job.key}
                            job={job}
                            onRun={handleRun}
                            onViewHistory={handleViewHistory}
                            onDelete={handleDelete}
                            onRefresh={fetchData}
                        />
                    ))
                ) : (
                    <div className="col-span-2 py-20 flex flex-col items-center justify-center text-center space-y-4">
                        <div className="p-4 rounded-full bg-white/5 border border-border/40">
                            <Clock className="w-8 h-8 text-muted-foreground" />
                        </div>
                        <div>
                            <h3 className="text-lg font-semibold">No active jobs</h3>
                            <p className="text-sm text-muted-foreground max-w-xs mx-auto">
                                You haven't configured any scheduled tasks yet. Click <strong>Create Job</strong> to add one.
                            </p>
                        </div>
                        <button
                            onClick={() => setShowCreateModal(true)}
                            className="flex items-center gap-2 px-5 py-2.5 rounded-xl bg-primary/10 text-primary border border-primary/20 text-sm font-medium hover:bg-primary/20 transition-all"
                        >
                            <Plus className="w-4 h-4" />
                            Create First Job
                        </button>
                    </div>
                )}
            </div>

            {/* History Sidebar/Modal Overlay */}
            <AnimatePresence>
                {historyJob && (
                    <div className="fixed inset-0 z-50 flex justify-end">
                        <motion.div
                            initial={{ opacity: 0 }}
                            animate={{ opacity: 1 }}
                            exit={{ opacity: 0 }}
                            onClick={() => setHistoryJob(null)}
                            className="absolute inset-0 bg-black/40 backdrop-blur-sm"
                        />
                        <motion.div
                            initial={{ x: "100%" }}
                            animate={{ x: 0 }}
                            exit={{ x: "100%" }}
                            className="relative w-full max-w-md bg-[#0D0D0E] border-l border-border/40 shadow-2xl flex flex-col"
                        >
                            <div className="p-6 border-b border-border/40 flex items-center justify-between">
                                <div className="flex items-center gap-3">
                                    <HistoryIcon className="w-5 h-5 text-primary" />
                                    <h2 className="text-lg font-semibold truncate">{historyJob} History</h2>
                                </div>
                                <button
                                    onClick={() => setHistoryJob(null)}
                                    className="p-2 rounded-lg hover:bg-white/5 text-muted-foreground transition-colors"
                                >
                                    <X className="w-5 h-5" />
                                </button>
                            </div>

                            <div className="flex-1 overflow-y-auto p-6 space-y-4">
                                {history.length > 0 ? (
                                    history.map((entry: openclaw.RoutineAuditEntry, idx) => (
                                        <div key={idx} className="p-4 rounded-xl bg-white/[0.03] border border-white/5">
                                            <div className="flex items-center justify-between mb-2">
                                                <span className="text-[10px] font-mono text-muted-foreground">
                                                    {entry.started_at ? new Date(entry.started_at).toLocaleString() : 'Just now'}
                                                </span>
                                                <div className={cn(
                                                    "px-2 py-0.5 rounded-full text-[9px] font-bold uppercase",
                                                    entry.outcome === 'success' ? "bg-green-500/10 text-green-500" :
                                                        entry.outcome === 'failure' ? "bg-red-500/10 text-red-400" :
                                                            "bg-amber-500/10 text-muted-foreground"
                                                )}>
                                                    {entry.outcome}
                                                </div>
                                            </div>
                                            {entry.duration_ms && (
                                                <div className="flex items-center gap-2 text-xs text-muted-foreground">
                                                    <Clock className="w-3 h-3" />
                                                    {entry.duration_ms}ms
                                                </div>
                                            )}
                                            {entry.error && (
                                                <div className="mt-3 p-2 rounded bg-red-500/5 border border-red-500/10 text-[10px] text-red-400 font-mono">
                                                    {entry.error}
                                                </div>
                                            )}
                                        </div>
                                    ))
                                ) : (
                                    <div className="h-full flex flex-col items-center justify-center text-center opacity-50">
                                        <HistoryIcon className="w-8 h-8 mb-2" />
                                        <p className="text-sm">No execution history found</p>
                                    </div>
                                )}
                            </div>
                        </motion.div>
                    </div>
                )}
            </AnimatePresence>

            {/* Quick Tips */}
            <div className="p-6 rounded-2xl border bg-amber-500/5 border-amber-500/10 flex gap-4">
                <div className="p-2 bg-amber-500/10 rounded-xl h-fit">
                    <Zap className="w-5 h-5 text-amber-500" />
                </div>
                <div>
                    <h4 className="text-sm font-semibold text-amber-500 uppercase tracking-wider">Background Execution</h4>
                    <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                        Cron jobs run in the background on the OpenClaw node.
                        They can trigger tools, send notifications, or clean up local storage without active UI sessions.
                        Jobs created here are stored in ThinClaw's RoutineStore and survive engine restarts.
                    </p>
                </div>
            </div>

            {/* Create Job Modal */}
            <AnimatePresence>
                {showCreateModal && (
                    <CreateJobModal
                        onClose={() => setShowCreateModal(false)}
                        onCreated={() => { setIsLoading(true); fetchData(); }}
                    />
                )}
            </AnimatePresence>
        </motion.div>
    );
}
