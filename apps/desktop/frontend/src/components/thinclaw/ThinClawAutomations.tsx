import { AnimatePresence, motion } from 'framer-motion';
import {
    AlertCircle, ArrowRight, CheckCircle2, Clock, History as HistoryIcon, Plus,
    RefreshCw, Search, X, Zap,
} from 'lucide-react';

import { cn } from '../../lib/utils';
import type * as thinclaw from '../../lib/thinclaw';
import { CreateJobModal } from './automations/CreateJobModal';
import { JobCard } from './automations/JobCard';
import { useAutomations } from './automations/use-automations';
import { ThinClawModeBadge } from './ThinClawModeBadge';

export function ThinClawAutomations() {
    const {
        jobs, historyJob, setHistoryJob, history, isLoading, setIsLoading, showCreateModal,
        setShowCreateModal, runtimeStatus, cronExpr, setCronExpr, lintResult, lintError,
        isLinting, fetchData, handleRun, handleDelete, handleToggle, handleViewHistory,
        handleLintCron,
    } = useAutomations();

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
                    <ThinClawModeBadge status={runtimeStatus} />
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
                        className="flex-1 px-4 py-2.5 rounded-xl bg-white/3 border border-border/40 text-sm font-mono placeholder:text-muted-foreground/40 focus:outline-hidden focus:ring-2 focus:ring-primary/30 focus:border-primary/40 transition-all"
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
                            <AlertCircle className="w-4 h-4 text-red-400 mt-0.5 shrink-0" />
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
                        <div key={i} className="h-48 rounded-2xl border border-white/5 bg-white/2 animate-pulse" />
                    ))
                ) : jobs.length > 0 ? (
                    jobs.map(job => (
                        <JobCard
                            key={job.key}
                            job={job}
                            onRun={handleRun}
                            onViewHistory={handleViewHistory}
                            onDelete={handleDelete}
                            onToggle={handleToggle}
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
                            className="absolute inset-0 bg-black/40 backdrop-blur-xs"
                        />
                        <motion.div
                            initial={{ x: "100%" }}
                            animate={{ x: 0 }}
                            exit={{ x: "100%" }}
                            className="relative w-full max-w-md bg-surface-elevated border-l border-border/40 shadow-2xl flex flex-col"
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
                                    history.map((entry: thinclaw.RoutineAuditEntry, idx) => (
                                        <div key={idx} className="p-4 rounded-xl bg-white/3 border border-white/5">
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
                        Cron jobs run in the background on the ThinClaw node.
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
