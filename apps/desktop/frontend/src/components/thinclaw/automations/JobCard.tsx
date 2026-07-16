import { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import {
    Activity, AlertCircle, Calendar, CheckCircle2, Clock, Gauge, Heart,
    History as HistoryIcon, Play, Timer, ToggleLeft, ToggleRight, X,
} from 'lucide-react';
import { toast } from 'sonner';

import { cn } from '../../../lib/utils';
import * as thinclaw from '../../../lib/thinclaw';
import { INTERVAL_PRESETS, parseIntervalMinutes } from './schedule';

export interface JobCardProps {
    job: thinclaw.CronJob;
    onRun: (key: string) => void;
    onViewHistory: (key: string) => void;
    onDelete: (key: string, name: string) => void;
    onToggle: (key: string, enabled: boolean, name: string) => void;
    onRefresh?: () => void;
}

export function JobCard({ job, onRun, onViewHistory, onDelete, onToggle, onRefresh }: JobCardProps) {
    const [confirmingDelete, setConfirmingDelete] = useState(false);
    const [updatingInterval, setUpdatingInterval] = useState(false);

    const currentInterval = parseIntervalMinutes(job.schedule);

    const handleSetInterval = async (minutes: number) => {
        if (minutes === currentInterval) return;
        setUpdatingInterval(true);
        try {
            await thinclaw.setHeartbeatInterval(minutes);
            toast.success(`Heartbeat interval set to ${minutes < 60 ? `${minutes} min` : `${minutes / 60}h`}`);
            onRefresh?.();
        } catch (e) {
            toast.error(`Failed to update interval: ${String(e)}`);
        } finally {
            setUpdatingInterval(false);
        }
    };
    return (
        <div className="p-5 rounded-2xl border bg-card/30 backdrop-blur-md shadow-xs border-border/40 group relative overflow-hidden">
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
                        disabled={job.enabled === false}
                        className="p-1.5 rounded-md hover:bg-white/5 text-primary transition-colors"
                        title={job.enabled === false ? 'Enable routine before running' : 'Run Now'}
                    >
                        <Play className="w-4 h-4 fill-current" />
                    </button>
                    <button
                        onClick={() => onToggle(job.key, !(job.enabled !== false), job.name ?? job.key)}
                        className={cn(
                            "p-1.5 rounded-md transition-colors",
                            job.enabled === false
                                ? "hover:bg-emerald-500/10 text-muted-foreground hover:text-emerald-400"
                                : "hover:bg-amber-500/10 text-muted-foreground hover:text-amber-400",
                        )}
                        title={job.enabled === false ? 'Enable Routine' : 'Disable Routine'}
                    >
                        {job.enabled === false ? <ToggleLeft className="w-4 h-4" /> : <ToggleRight className="w-4 h-4" />}
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
                                        ? 'bg-rose-500/15 text-rose-400 border-rose-500/30 shadow-xs shadow-rose-500/10'
                                        : 'bg-white/3 text-muted-foreground hover:bg-white/5 border-border/30 hover:border-rose-500/20',
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
                    {job.enabled === false ? (
                        <AlertCircle className="w-3.5 h-3.5 text-amber-500" />
                    ) : job.lastStatus === 'ok' ? (
                        <CheckCircle2 className="w-3.5 h-3.5 text-green-500" />
                    ) : job.lastStatus === 'error' ? (
                        <AlertCircle className="w-3.5 h-3.5 text-red-500" />
                    ) : (
                        <CircleIcon className="w-3.5 h-3.5 text-muted-foreground/30" />
                    )}
                    <span className="text-[10px] text-muted-foreground uppercase font-bold tracking-tight">
                        {job.enabled === false ? 'Disabled' : `Last Exit: ${job.lastStatus || 'Never'}`}
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
                        className="absolute inset-0 bg-red-950/95 backdrop-blur-xs rounded-2xl flex flex-col items-center justify-center gap-3 z-10 p-4"
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
