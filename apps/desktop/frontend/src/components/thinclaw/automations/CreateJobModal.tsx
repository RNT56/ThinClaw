import { useEffect, useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Activity, AlertCircle, CheckCircle2, FileText, Plus, RefreshCw, Terminal, Timer, X } from 'lucide-react';
import { toast } from 'sonner';

import { cn } from '../../../lib/utils';
import * as thinclaw from '../../../lib/thinclaw';
import { SCHEDULE_PRESETS } from './schedule';

// ── Create Job Modal ─────────────────────────────────────────────────

export interface CreateJobModalProps {
    onClose: () => void;
    onCreated: () => void;
}

export function CreateJobModal({ onClose, onCreated }: CreateJobModalProps) {
    const [name, setName] = useState('');
    const [description, setDescription] = useState('');
    const [triggerType, setTriggerType] = useState<thinclaw.RoutineTriggerType>('cron');
    const [schedule, setSchedule] = useState('0 0 * * * * *');
    const [task, setTask] = useState('');
    const [isSubmitting, setIsSubmitting] = useState(false);
    const [lintResult, setLintResult] = useState<thinclaw.CronLintResult | null>(null);
    const [lintError, setLintError] = useState<string | null>(null);
    const [isLinting, setIsLinting] = useState(false);

    // Auto-lint when schedule changes
    useEffect(() => {
        if (!schedule.trim()) { setLintResult(null); setLintError(null); return; }
        const timer = setTimeout(async () => {
            setIsLinting(true);
            setLintError(null);
            try {
                const r = await thinclaw.lintCronExpression(schedule.trim());
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
            await thinclaw.createRoutine(
                name.trim(),
                description.trim(),
                schedule.trim(),
                task.trim(),
                triggerType,
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

    const inputCls = 'w-full h-9 rounded-lg border border-border/40 bg-white/3 px-3 text-sm text-zinc-200 placeholder:text-muted-foreground/40 focus:outline-hidden focus:ring-2 focus:ring-primary/30 focus:border-primary/40 transition-all';

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
            <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                exit={{ opacity: 0 }}
                className="absolute inset-0 bg-black/60 backdrop-blur-xs"
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
                <div className="flex items-center justify-between px-6 py-4 border-b border-border/40 bg-white/2">
                    <div className="flex items-center gap-3">
                        <div className="p-2 bg-primary/10 rounded-lg">
                            <Timer className="w-4 h-4 text-primary" />
                        </div>
                        <div>
                            <h2 className="text-base font-bold">Create Routine</h2>
                            <p className="text-[11px] text-muted-foreground">Schedule an agent job or heartbeat system event</p>
                        </div>
                    </div>
                    <button aria-label="Close create routine" onClick={onClose} className="p-1.5 hover:bg-white/10 rounded-lg transition-colors text-muted-foreground hover:text-white">
                        <X className="w-4 h-4" />
                    </button>
                </div>

                {/* Form */}
                <form onSubmit={handleSubmit} className="flex-1 overflow-y-auto">
                    <div className="px-6 py-5 space-y-5">
                        {/* Name */}
                        <div className="space-y-1.5">
                            <label htmlFor="routine-name" className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70">
                                Job Name <span className="text-red-400">*</span>
                            </label>
                            <input
                                id="routine-name"
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
                            <label htmlFor="routine-description" className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70">
                                Description
                            </label>
                            <input
                                id="routine-description"
                                type="text"
                                value={description}
                                onChange={e => setDescription(e.target.value)}
                                placeholder="What does this job do?"
                                className={inputCls}
                            />
                        </div>

                        {/* Trigger type */}
                        <fieldset className="space-y-2">
                            <legend className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70">
                                Trigger Type
                            </legend>
                            <div className="grid grid-cols-2 gap-2" role="radiogroup" aria-label="Routine trigger type">
                                {([
                                    ['cron', Timer, 'Agent job', 'Run the task as a full agent job.'],
                                    ['system_event', Activity, 'System event', 'Queue a message for the next heartbeat.'],
                                ] as const).map(([value, Icon, label, help]) => (
                                    <button
                                        key={value}
                                        type="button"
                                        role="radio"
                                        aria-checked={triggerType === value}
                                        onClick={() => setTriggerType(value)}
                                        className={cn(
                                            'rounded-xl border p-3 text-left transition-all',
                                            triggerType === value
                                                ? 'border-primary/40 bg-primary/10 text-zinc-100'
                                                : 'border-border/30 bg-white/2 text-muted-foreground hover:bg-white/5',
                                        )}
                                    >
                                        <span className="flex items-center gap-2 text-xs font-semibold">
                                            <Icon className="h-3.5 w-3.5" />
                                            {label}
                                        </span>
                                        <span className="mt-1 block text-[10px] leading-relaxed opacity-70">{help}</span>
                                    </button>
                                ))}
                            </div>
                        </fieldset>

                        {/* Schedule presets */}
                        <div className="space-y-2">
                            <label htmlFor="routine-schedule" className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70">
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
                                                : 'bg-white/3 text-muted-foreground hover:bg-white/5 border-border/30',
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
                                    id="routine-schedule"
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

                        {/* Task prompt / system event message */}
                        <div className="space-y-1.5">
                            <label htmlFor="routine-task" className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/70 flex items-center gap-1.5">
                                {triggerType === 'cron' ? <Terminal className="w-3 h-3" /> : <Activity className="w-3 h-3" />}
                                {triggerType === 'cron' ? 'Agent Task Prompt' : 'System Event Message'} <span className="text-red-400">*</span>
                            </label>
                            <textarea
                                id="routine-task"
                                value={task}
                                onChange={e => setTask(e.target.value)}
                                placeholder={triggerType === 'cron'
                                    ? "Describe what the agent should do when this job fires. E.g: 'Summarize yesterday's logs and write a report.'"
                                    : "Describe the reminder or check to queue for the heartbeat. E.g: 'Review stalled pull requests and alert me if action is needed.'"}
                                rows={4}
                                className={cn(inputCls, 'h-auto py-2 resize-y text-xs leading-relaxed')}
                                required
                            />
                            <p className="text-[10px] text-muted-foreground/50 flex items-center gap-1.5">
                                <FileText className="w-3 h-3" />
                                {triggerType === 'cron'
                                    ? 'This prompt starts a full agent job when the schedule fires.'
                                    : 'This message enters the heartbeat queue when the schedule fires.'}
                            </p>
                        </div>
                    </div>

                    {/* Footer */}
                    <div className="px-6 py-4 border-t border-border/40 bg-white/1 flex items-center justify-end gap-3">
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
