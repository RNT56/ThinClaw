import { useCallback, useEffect, useMemo, useState } from 'react';
import { motion } from 'framer-motion';
import {
    AlertTriangle,
    CheckCircle2,
    Circle,
    FileText,
    Folder,
    MessageSquarePlus,
    Play,
    RefreshCw,
    RotateCcw,
    Square,
    XCircle,
} from 'lucide-react';
import { toast } from 'sonner';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';

function stateTone(state?: string) {
    switch (state) {
        case 'running':
        case 'in_progress':
        case 'creating':
        case 'pending':
            return 'text-blue-400 bg-blue-500/10 border-blue-500/20';
        case 'completed':
        case 'accepted':
        case 'submitted':
            return 'text-emerald-400 bg-emerald-500/10 border-emerald-500/20';
        case 'failed':
        case 'stuck':
        case 'interrupted':
            return 'text-red-400 bg-red-500/10 border-red-500/20';
        case 'cancelled':
        case 'abandoned':
            return 'text-amber-400 bg-amber-500/10 border-amber-500/20';
        default:
            return 'text-muted-foreground bg-white/[0.03] border-white/5';
    }
}

function formatDate(value?: string | null) {
    if (!value) return 'Never';
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function reasonFromError(err: unknown) {
    return err instanceof Error ? err.message : String(err);
}

export function OpenClawJobs() {
    const [jobs, setJobs] = useState<openclaw.OpenClawJob[]>([]);
    const [summary, setSummary] = useState<openclaw.OpenClawJobSummary | null>(null);
    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [detail, setDetail] = useState<openclaw.OpenClawJobDetail | null>(null);
    const [events, setEvents] = useState<openclaw.OpenClawJobEvent[]>([]);
    const [files, setFiles] = useState<openclaw.OpenClawJobFileEntry[]>([]);
    const [filePath, setFilePath] = useState('');
    const [fileContent, setFileContent] = useState('');
    const [prompt, setPrompt] = useState('');
    const [unavailable, setUnavailable] = useState<Record<string, string>>({});
    const [error, setError] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(true);

    const selectedJob = useMemo(
        () => jobs.find((job) => job.id === selectedId) ?? null,
        [jobs, selectedId],
    );

    const loadList = useCallback(async () => {
        setIsLoading(true);
        setError(null);
        try {
            const [list, nextSummary] = await Promise.all([
                openclaw.listJobs(),
                openclaw.getJobsSummary().catch(() => null),
            ]);
            setJobs(list.jobs ?? []);
            setUnavailable(list.unavailable ?? {});
            setSummary(nextSummary);
            setSelectedId((current) => current ?? list.jobs?.[0]?.id ?? null);
        } catch (err) {
            setError(reasonFromError(err));
            setJobs([]);
        } finally {
            setIsLoading(false);
        }
    }, []);

    const loadDetail = useCallback(async (jobId: string) => {
        try {
            const [nextDetail, eventResponse] = await Promise.all([
                openclaw.getJobDetail(jobId),
                openclaw.getJobEvents(jobId).catch((err) => ({
                    job_id: jobId,
                    events: [],
                    unavailable_reason: reasonFromError(err),
                })),
            ]);
            setDetail(nextDetail);
            setEvents(eventResponse.events ?? []);
        } catch (err) {
            setDetail(null);
            setEvents([]);
            toast.error(reasonFromError(err));
        }
    }, []);

    const loadFiles = useCallback(async (jobId: string, path = '') => {
        try {
            const response = await openclaw.listJobFiles(jobId, path);
            setFiles(response.entries ?? []);
            setFilePath(path);
            setFileContent('');
        } catch (err) {
            setFiles([]);
            toast.error(reasonFromError(err));
        }
    }, []);

    useEffect(() => {
        loadList();
        const interval = setInterval(loadList, 10000);
        return () => clearInterval(interval);
    }, [loadList]);

    useEffect(() => {
        if (!selectedId) {
            setDetail(null);
            setEvents([]);
            setFiles([]);
            return;
        }
        loadDetail(selectedId);
    }, [selectedId, loadDetail]);

    const handleAction = async (action: 'cancel' | 'restart' | 'prompt' | 'done') => {
        if (!selectedId) return;
        try {
            if (action === 'cancel') await openclaw.cancelJob(selectedId);
            if (action === 'restart') await openclaw.restartJob(selectedId);
            if (action === 'prompt') await openclaw.promptJob(selectedId, prompt, false);
            if (action === 'done') await openclaw.promptJob(selectedId, null, true);
            if (action === 'prompt') setPrompt('');
            toast.success('Job command submitted');
            await Promise.all([loadList(), loadDetail(selectedId)]);
        } catch (err) {
            toast.error(reasonFromError(err));
        }
    };

    const handleReadFile = async (path: string) => {
        if (!selectedId) return;
        try {
            const response = await openclaw.readJobFile(selectedId, path);
            setFileContent(response.content);
            setFilePath(response.path);
        } catch (err) {
            toast.error(reasonFromError(err));
        }
    };

    const stats = summary ?? {
        total: jobs.length,
        pending: 0,
        in_progress: 0,
        completed: 0,
        failed: 0,
        cancelled: 0,
        interrupted: 0,
        stuck: 0,
    };

    return (
        <motion.div className="flex-1 overflow-y-auto p-8 space-y-6" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
            <div className="flex items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-lg bg-blue-500/10 border border-blue-500/20">
                        <Play className="w-5 h-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Jobs</h1>
                        <p className="text-xs text-muted-foreground">Background execution, events, files, and interactive prompts</p>
                    </div>
                </div>
                <button
                    onClick={loadList}
                    className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                >
                    <RefreshCw className={cn('w-4 h-4', isLoading && 'animate-spin')} />
                </button>
            </div>

            {error && (
                <div className="flex items-start gap-3 rounded-lg border border-amber-500/20 bg-amber-500/10 p-4 text-sm text-amber-200">
                    <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" />
                    <span>{error}</span>
                </div>
            )}

            <div className="grid grid-cols-2 lg:grid-cols-6 gap-3">
                {[
                    ['Total', stats.total],
                    ['Running', stats.in_progress],
                    ['Pending', stats.pending],
                    ['Done', stats.completed],
                    ['Failed', stats.failed + stats.stuck + stats.interrupted],
                    ['Cancelled', stats.cancelled],
                ].map(([label, value]) => (
                    <div key={label} className="rounded-lg border border-border/40 bg-card/30 p-4">
                        <p className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">{label}</p>
                        <p className="text-2xl font-bold tabular-nums mt-1">{value}</p>
                    </div>
                ))}
            </div>

            <div className="grid grid-cols-1 xl:grid-cols-[360px_1fr] gap-6 min-h-[520px]">
                <div className="rounded-lg border border-border/40 bg-card/30 overflow-hidden">
                    <div className="px-4 py-3 border-b border-border/40 text-xs font-bold uppercase tracking-widest text-muted-foreground">
                        Queue
                    </div>
                    <div className="max-h-[640px] overflow-y-auto">
                        {jobs.length === 0 ? (
                            <div className="p-8 text-center text-sm text-muted-foreground">
                                {isLoading ? 'Loading jobs...' : 'No jobs found'}
                            </div>
                        ) : jobs.map((job) => (
                            <button
                                key={job.id}
                                onClick={() => setSelectedId(job.id)}
                                className={cn(
                                    'w-full text-left px-4 py-3 border-b border-border/30 hover:bg-white/[0.03] transition-colors',
                                    selectedId === job.id && 'bg-primary/10',
                                )}
                            >
                                <div className="flex items-start justify-between gap-3">
                                    <div className="min-w-0">
                                        <p className="text-sm font-semibold truncate">{job.title || job.id}</p>
                                        <p className="text-[10px] text-muted-foreground truncate font-mono mt-1">{job.id}</p>
                                    </div>
                                    <span className={cn('text-[10px] px-2 py-1 rounded-md border shrink-0', stateTone(job.state))}>
                                        {job.state}
                                    </span>
                                </div>
                                <p className="text-[10px] text-muted-foreground mt-2">{formatDate(job.created_at)}</p>
                            </button>
                        ))}
                    </div>
                </div>

                <div className="space-y-6">
                    <div className="rounded-lg border border-border/40 bg-card/30 p-5">
                        <div className="flex flex-wrap items-start justify-between gap-4">
                            <div className="min-w-0">
                                <p className="text-[10px] uppercase font-bold tracking-widest text-muted-foreground">Selected Job</p>
                                <h2 className="text-lg font-bold mt-1 truncate">{detail?.title ?? selectedJob?.title ?? 'No job selected'}</h2>
                                <p className="text-xs text-muted-foreground mt-1 font-mono truncate">{selectedId ?? 'Select a job from the queue'}</p>
                            </div>
                            {detail && (
                                <span className={cn('text-xs px-2.5 py-1 rounded-md border', stateTone(detail.state))}>{detail.state}</span>
                            )}
                        </div>

                        {detail && (
                            <>
                                <p className="text-sm text-muted-foreground mt-4 whitespace-pre-wrap">{detail.description || 'No description'}</p>
                                <div className="grid grid-cols-2 lg:grid-cols-4 gap-3 mt-4">
                                    <div>
                                        <p className="text-[10px] uppercase font-bold text-muted-foreground">Backend</p>
                                        <p className="text-xs mt-1">{detail.execution_backend ?? 'unknown'}</p>
                                    </div>
                                    <div>
                                        <p className="text-[10px] uppercase font-bold text-muted-foreground">Runtime</p>
                                        <p className="text-xs mt-1">{detail.runtime_mode ?? detail.runtime_family ?? 'unknown'}</p>
                                    </div>
                                    <div>
                                        <p className="text-[10px] uppercase font-bold text-muted-foreground">Started</p>
                                        <p className="text-xs mt-1">{formatDate(detail.started_at)}</p>
                                    </div>
                                    <div>
                                        <p className="text-[10px] uppercase font-bold text-muted-foreground">Elapsed</p>
                                        <p className="text-xs mt-1">{detail.elapsed_secs == null ? 'n/a' : `${detail.elapsed_secs}s`}</p>
                                    </div>
                                </div>
                                <div className="flex flex-wrap gap-2 mt-5">
                                    <button onClick={() => handleAction('cancel')} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs bg-red-500/10 text-red-300 border border-red-500/20 hover:bg-red-500/20">
                                        <Square className="w-3.5 h-3.5" />
                                        Cancel
                                    </button>
                                    <button onClick={() => handleAction('restart')} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs bg-white/[0.03] border border-white/5 hover:bg-white/5" title={unavailable.restart}>
                                        <RotateCcw className="w-3.5 h-3.5" />
                                        Restart
                                    </button>
                                    <button onClick={() => loadFiles(detail.id)} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs bg-white/[0.03] border border-white/5 hover:bg-white/5" title={unavailable.files}>
                                        <Folder className="w-3.5 h-3.5" />
                                        Files
                                    </button>
                                </div>
                            </>
                        )}
                    </div>

                    <div className="grid grid-cols-1 2xl:grid-cols-2 gap-6">
                        <div className="rounded-lg border border-border/40 bg-card/30 p-5">
                            <div className="flex items-center justify-between mb-4">
                                <h3 className="text-sm font-bold">Events</h3>
                                <button disabled={!selectedId} onClick={() => selectedId && loadDetail(selectedId)} className="p-1.5 rounded-md hover:bg-white/5 disabled:opacity-40">
                                    <RefreshCw className="w-3.5 h-3.5" />
                                </button>
                            </div>
                            <div className="space-y-2 max-h-72 overflow-y-auto">
                                {events.length === 0 ? (
                                    <p className="text-xs text-muted-foreground">No events recorded.</p>
                                ) : events.map((event, index) => (
                                    <div key={event.id ?? index} className="rounded-md border border-white/5 bg-black/20 p-3">
                                        <div className="flex items-center justify-between gap-2">
                                            <p className="text-xs font-semibold">{event.event_type}</p>
                                            <p className="text-[10px] text-muted-foreground">{formatDate(event.created_at)}</p>
                                        </div>
                                        {event.data != null && (
                                            <pre className="text-[10px] text-muted-foreground mt-2 overflow-x-auto">{JSON.stringify(event.data, null, 2)}</pre>
                                        )}
                                    </div>
                                ))}
                            </div>
                        </div>

                        <div className="rounded-lg border border-border/40 bg-card/30 p-5">
                            <h3 className="text-sm font-bold mb-4">Prompt</h3>
                            <textarea
                                value={prompt}
                                onChange={(event) => setPrompt(event.target.value)}
                                className="w-full min-h-24 rounded-lg bg-black/20 border border-white/5 px-3 py-2 text-sm outline-none focus:border-primary/40 resize-y"
                                placeholder={unavailable.prompt ?? 'Send a follow-up prompt to an interactive job'}
                            />
                            <div className="flex gap-2 mt-3">
                                <button disabled={!prompt.trim()} onClick={() => handleAction('prompt')} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs bg-primary/10 text-primary border border-primary/20 hover:bg-primary/20 disabled:opacity-40">
                                    <MessageSquarePlus className="w-3.5 h-3.5" />
                                    Send
                                </button>
                                <button onClick={() => handleAction('done')} className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs bg-white/[0.03] border border-white/5 hover:bg-white/5">
                                    <CheckCircle2 className="w-3.5 h-3.5" />
                                    Done
                                </button>
                            </div>
                        </div>
                    </div>

                    <div className="rounded-lg border border-border/40 bg-card/30 p-5">
                        <div className="flex items-center justify-between mb-4">
                            <h3 className="text-sm font-bold">Files</h3>
                            <p className="text-[10px] text-muted-foreground font-mono">{filePath || '/'}</p>
                        </div>
                        <div className="grid grid-cols-1 lg:grid-cols-[320px_1fr] gap-4">
                            <div className="space-y-1 max-h-80 overflow-y-auto">
                                {filePath && (
                                    <button onClick={() => loadFiles(selectedId!, filePath.split('/').slice(0, -1).join('/'))} className="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-xs hover:bg-white/5">
                                        <Folder className="w-3.5 h-3.5" />
                                        ..
                                    </button>
                                )}
                                {files.length === 0 ? (
                                    <p className="text-xs text-muted-foreground">{unavailable.files ?? 'No files loaded.'}</p>
                                ) : files.map((entry) => (
                                    <button
                                        key={entry.path}
                                        onClick={() => entry.is_dir ? loadFiles(selectedId!, entry.path) : handleReadFile(entry.path)}
                                        className="w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-xs hover:bg-white/5 text-left"
                                    >
                                        {entry.is_dir ? <Folder className="w-3.5 h-3.5 text-blue-400" /> : <FileText className="w-3.5 h-3.5 text-muted-foreground" />}
                                        <span className="truncate">{entry.name}</span>
                                    </button>
                                ))}
                            </div>
                            <pre className="min-h-40 max-h-80 overflow-auto rounded-lg bg-black/20 border border-white/5 p-3 text-[11px] text-muted-foreground whitespace-pre-wrap">
                                {fileContent || 'Select a file to inspect its content.'}
                            </pre>
                        </div>
                    </div>

                    <div className="rounded-lg border border-border/40 bg-card/30 p-5">
                        <h3 className="text-sm font-bold mb-4">Transitions</h3>
                        <div className="space-y-2">
                            {(detail?.transitions ?? []).length === 0 ? (
                                <p className="text-xs text-muted-foreground">No transitions recorded.</p>
                            ) : detail!.transitions!.map((transition, index) => (
                                <div key={`${transition.timestamp}-${index}`} className="flex items-center gap-3 text-xs">
                                    {transition.to === 'failed' ? <XCircle className="w-3.5 h-3.5 text-red-400" /> : <Circle className="w-3.5 h-3.5 text-muted-foreground" />}
                                    <span className="font-mono text-muted-foreground">{formatDate(transition.timestamp)}</span>
                                    <span>{transition.from} -&gt; {transition.to}</span>
                                    {transition.reason && <span className="text-muted-foreground truncate">{transition.reason}</span>}
                                </div>
                            ))}
                        </div>
                    </div>
                </div>
            </div>
        </motion.div>
    );
}
