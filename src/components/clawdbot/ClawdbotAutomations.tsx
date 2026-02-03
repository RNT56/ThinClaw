import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
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
    MoreHorizontal,
    X,
    Zap
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as clawdbot from '../../lib/clawdbot';
import { toast } from 'sonner';

interface JobCardProps {
    job: clawdbot.CronJob;
    onRun: (key: string) => void;
    onViewHistory: (key: string) => void;
}

function JobCard({ job, onRun, onViewHistory }: JobCardProps) {
    return (
        <div className="p-5 rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm border-white/10 group">
            <div className="flex items-start justify-between mb-4">
                <div className="flex items-center gap-3">
                    <div className="p-2 bg-primary/10 rounded-lg">
                        <Timer className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h3 className="font-semibold">{job.key}</h3>
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
                    <button className="p-1.5 rounded-md hover:bg-white/5 text-muted-foreground transition-colors">
                        <MoreHorizontal className="w-4 h-4" />
                    </button>
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

            <div className="mt-4 pt-4 border-t border-white/5 flex items-center justify-between">
                <div className="flex items-center gap-2">
                    {job.lastStatus === 'ok' ? (
                        <CheckCircle2 className="w-3.5 h-3.5 text-green-500" />
                    ) : job.lastStatus === 'error' ? (
                        <AlertCircle className="w-3.5 h-3.5 text-red-500" />
                    ) : (
                        <Circle className="w-3.5 h-3.5 text-muted-foreground/30" />
                    )}
                    <span className="text-[10px] text-muted-foreground uppercase font-bold tracking-tight">
                        Last Exit: {job.lastStatus || 'Never'}
                    </span>
                </div>
                <span className="text-[10px] text-muted-foreground font-mono">
                    {job.lastRun ? `Ran ${job.lastRun}` : 'No history'}
                </span>
            </div>
        </div>
    );
}

// Minimal circle for status
function Circle({ className }: { className?: string }) {
    return (
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" className={className}>
            <circle cx="12" cy="12" r="10" />
        </svg>
    );
}

export function ClawdbotAutomations() {
    const [jobs, setJobs] = useState<clawdbot.CronJob[]>([]);
    const [historyJob, setHistoryJob] = useState<string | null>(null);
    const [history, setHistory] = useState<any[]>([]);
    const [isLoading, setIsLoading] = useState(true);

    const fetchData = async () => {
        try {
            const data = await clawdbot.getClawdbotCronList();
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

    const handleRun = async (key: string) => {
        try {
            toast.promise(clawdbot.runClawdbotCron(key), {
                loading: `Triggering ${key}...`,
                success: `${key} triggered successfully`,
                error: (err) => `Failed to run ${key}: ${err}`
            });
        } catch (e) { }
    };

    const handleViewHistory = async (key: string) => {
        setHistoryJob(key);
        setHistory([]);
        try {
            const data = await clawdbot.getClawdbotCronHistory(key, 10);
            setHistory(Array.isArray(data) ? data : []);
        } catch (e) {
            toast.error(`Failed to fetch history for ${key}`);
        }
    };

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
                    <button className="flex items-center gap-2 px-4 py-2 rounded-lg bg-primary text-primary-foreground text-sm font-medium hover:opacity-90 transition-all shadow-lg shadow-primary/20">
                        <Plus className="w-4 h-4" />
                        Create Job
                    </button>
                    <button
                        onClick={() => {
                            setIsLoading(true);
                            fetchData();
                        }}
                        className="p-2.5 rounded-lg bg-card border border-white/10 hover:bg-white/5 transition-colors"
                    >
                        <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                    </button>
                </div>
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
                        />
                    ))
                ) : (
                    <div className="col-span-2 py-20 flex flex-col items-center justify-center text-center space-y-4">
                        <div className="p-4 rounded-full bg-white/5 border border-white/10">
                            <Clock className="w-8 h-8 text-muted-foreground" />
                        </div>
                        <div>
                            <h3 className="text-lg font-semibold">No active jobs</h3>
                            <p className="text-sm text-muted-foreground max-w-xs mx-auto">
                                You haven't configured any scheduled tasks for this node yet.
                            </p>
                        </div>
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
                            className="relative w-full max-w-md bg-[#0D0D0E] border-l border-white/10 shadow-2xl flex flex-col"
                        >
                            <div className="p-6 border-b border-white/10 flex items-center justify-between">
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
                                    history.map((entry, idx) => (
                                        <div key={idx} className="p-4 rounded-xl bg-white/[0.03] border border-white/5">
                                            <div className="flex items-center justify-between mb-2">
                                                <span className="text-[10px] font-mono text-muted-foreground">ID: {entry.id || idx}</span>
                                                <div className={cn(
                                                    "px-2 py-0.5 rounded-full text-[9px] font-bold uppercase",
                                                    entry.status === 'ok' ? "bg-green-500/10 text-green-500" : "bg-red-500/10 text-red-400"
                                                )}>
                                                    {entry.status}
                                                </div>
                                            </div>
                                            <div className="flex items-center gap-2 text-xs text-muted-foreground">
                                                <Clock className="w-3 h-3" />
                                                {entry.timestamp || 'Just now'}
                                            </div>
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
                    </p>
                </div>
            </div>
        </motion.div>
    );
}
