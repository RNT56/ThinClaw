import { useCallback, useEffect, useMemo, useState } from 'react';
import {
    FileText,
    Folder,
    MessageSquarePlus,
    RefreshCw,
    RotateCcw,
    Square,
} from 'lucide-react';
import { toast } from 'sonner';

import * as thinclaw from '../../lib/thinclaw';
import {
    AgentPageShell,
    AsyncState,
    Button,
    ConfirmDialog,
    MetricCard,
    Notice,
    StatusBadge,
    Surface,
} from '../ui';
import { normalizeAgentActionOutcome } from './action-outcome';
import { useOptionalAgentCockpit } from './AgentCockpitProvider';

function formatDate(value?: string | null) {
    if (!value) return 'Unknown';
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function reasonFromError(error: unknown): string {
    if (error && typeof error === 'object' && 'kind' in error) {
        const bridge = error as { kind?: string; message?: string; capability?: string; reason?: string; remediation?: string | null };
        if (bridge.kind === 'unavailable') {
            const head = [bridge.capability, bridge.reason].filter(Boolean).join(': ');
            return bridge.remediation ? `${head} — ${bridge.remediation}` : head || 'Unavailable';
        }
        if (bridge.kind === 'runtime' && bridge.message) return bridge.message;
    }
    return error instanceof Error ? error.message : String(error);
}

const CANCELLABLE_STATES = new Set(['queued', 'pending', 'running', 'in_progress', 'paused']);

/** Capability-aware Job center. Unsupported remote-only actions are not rendered. */
export function ThinClawJobs() {
    const cockpit = useOptionalAgentCockpit();
    const [jobs, setJobs] = useState<thinclaw.ThinClawJob[]>([]);
    const [summary, setSummary] = useState<thinclaw.ThinClawJobSummary | null>(null);
    const [selectedId, setSelectedId] = useState<string | null>(null);
    const [detail, setDetail] = useState<thinclaw.ThinClawJobDetail | null>(null);
    const [events, setEvents] = useState<thinclaw.ThinClawJobEvent[]>([]);
    const [files, setFiles] = useState<thinclaw.ThinClawJobFileEntry[]>([]);
    const [filePath, setFilePath] = useState('');
    const [fileContent, setFileContent] = useState('');
    const [prompt, setPrompt] = useState('');
    const [capabilities, setCapabilities] = useState<Record<string, boolean>>({});
    const [unavailable, setUnavailable] = useState<Record<string, string>>({});
    const [error, setError] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [activeAction, setActiveAction] = useState<string | null>(null);
    const [cancelDialogOpen, setCancelDialogOpen] = useState(false);

    const selectedJob = useMemo(() => jobs.find((job) => job.id === selectedId) ?? null, [jobs, selectedId]);

    const loadList = useCallback(async () => {
        setIsLoading(true);
        setError(null);
        try {
            const [list, nextSummary] = await Promise.all([
                thinclaw.listJobs(),
                thinclaw.getJobsSummary().catch(() => null),
            ]);
            const nextJobs = list.jobs ?? [];
            setJobs(nextJobs);
            setCapabilities(list.capabilities ?? {});
            setUnavailable(list.unavailable ?? {});
            setSummary(nextSummary);
            setSelectedId((current) => current && nextJobs.some((job) => job.id === current) ? current : nextJobs[0]?.id ?? null);
        } catch (caught) {
            setError(reasonFromError(caught));
            setJobs([]);
            setSelectedId(null);
        } finally {
            setIsLoading(false);
        }
    }, []);

    const loadDetail = useCallback(async (jobId: string) => {
        try {
            const [nextDetail, eventResponse] = await Promise.all([
                thinclaw.getJobDetail(jobId),
                thinclaw.getJobEvents(jobId).catch((caught) => ({ job_id: jobId, events: [], unavailable_reason: reasonFromError(caught) })),
            ]);
            setDetail(nextDetail);
            setEvents(eventResponse.events ?? []);
        } catch (caught) {
            setDetail(null);
            setEvents([]);
            toast.error(reasonFromError(caught));
        }
    }, []);

    const loadFiles = useCallback(async (jobId: string, path = '') => {
        try {
            const response = await thinclaw.listJobFiles(jobId, path);
            setFiles(response.entries ?? []);
            setFilePath(path);
            setFileContent('');
        } catch (caught) {
            setFiles([]);
            toast.error(reasonFromError(caught));
        }
    }, []);

    useEffect(() => {
        void loadList();
        const poll = window.setInterval(() => {
            if (document.visibilityState === 'visible') void loadList();
        }, 15_000);
        return () => window.clearInterval(poll);
    }, [loadList]);

    useEffect(() => {
        if (!selectedId) {
            setDetail(null);
            setEvents([]);
            setFiles([]);
            setFilePath('');
            setFileContent('');
            return;
        }
        void loadDetail(selectedId);
    }, [selectedId, loadDetail]);

    const can = (name: string) => capabilities[name] === true;
    const canCancel = Boolean(detail && can('cancel') && CANCELLABLE_STATES.has(detail.state));
    const canRestart = Boolean(detail && can('restart'));
    const canPrompt = Boolean(detail && can('prompt'));
    const canBrowseFiles = Boolean(detail && can('files'));

    const handleAction = async (action: 'cancel' | 'restart' | 'prompt' | 'done') => {
        if (!selectedId) return;
        const capability = action === 'done' ? 'prompt' : action;
        if (!can(capability)) {
            toast.error(unavailable[capability] ?? `${capability} is unavailable for this job`);
            return;
        }
        setActiveAction(action);
        try {
            let result: unknown;
            if (action === 'cancel') result = await thinclaw.cancelJob(selectedId);
            if (action === 'restart') result = await thinclaw.restartJob(selectedId);
            if (action === 'prompt') result = await thinclaw.promptJob(selectedId, prompt, false);
            if (action === 'done') result = await thinclaw.promptJob(selectedId, null, true);
            if (action === 'prompt') setPrompt('');
            const outcome = normalizeAgentActionOutcome(result, 'The job command returned without an outcome payload.');
            if (outcome.state === 'rejected') toast.error(outcome.message);
            else if (outcome.state === 'applied') toast.success(outcome.message);
            else toast.info(outcome.message);
            setCancelDialogOpen(false);
            await Promise.all([loadList(), loadDetail(selectedId)]);
        } catch (caught) {
            toast.error(reasonFromError(caught));
        } finally {
            setActiveAction(null);
        }
    };

    const handleReadFile = async (path: string) => {
        if (!selectedId) return;
        try {
            const response = await thinclaw.readJobFile(selectedId, path);
            setFileContent(response.content);
            setFilePath(response.path);
        } catch (caught) {
            toast.error(reasonFromError(caught));
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
    const sourceLabel = cockpit?.source === 'remote' ? 'Remote profile' : cockpit?.source === 'local' ? 'Local Core' : 'Profile';

    if (isLoading && jobs.length === 0) {
        return <AsyncState kind="loading" title="Loading jobs" className="flex-1" />;
    }

    return (
        <AgentPageShell
            eyebrow="Agent execution"
            title="Jobs"
            description="Inspect active and historical work. Controls only appear when the selected profile reports support."
            badge={<StatusBadge status={cockpit?.status?.engine_running ? 'Running' : cockpit?.error ? 'Unavailable' : 'Stopped'} label={sourceLabel} />}
            actions={<Button size="sm" variant="secondary" onClick={() => void loadList()} disabled={isLoading}><RefreshCw className={`size-3.5 ${isLoading ? 'animate-spin' : ''}`} aria-hidden="true" /> Refresh</Button>}
        >
            <div className="mx-auto flex w-full max-w-7xl flex-col gap-5">
                {error && <Notice tone="error" title="Jobs could not be loaded">{error}</Notice>}
                {Object.values(unavailable).some(Boolean) && (
                    <Notice tone="info" title="Capability-aware controls">
                        This profile does not expose every job control. Reasons are shown next to the relevant work area instead of failing after a click.
                    </Notice>
                )}

                <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-6">
                    <MetricCard label="Total" value={stats.total} />
                    <MetricCard label="Running" value={stats.in_progress} />
                    <MetricCard label="Pending" value={stats.pending} />
                    <MetricCard label="Completed" value={stats.completed} />
                    <MetricCard label="Failed" value={stats.failed + stats.stuck + stats.interrupted} />
                    <MetricCard label="Cancelled" value={stats.cancelled} />
                </div>

                <div className="grid min-h-[32rem] gap-5 xl:grid-cols-[minmax(16rem,22rem)_minmax(0,1fr)]">
                    <Surface className="min-h-0 overflow-hidden">
                        <div className="border-b border-surface-outline px-4 py-3 text-[10px] font-bold uppercase tracking-widest text-content-muted">Job queue</div>
                        <div className="max-h-[42rem] overflow-y-auto p-2">
                            {jobs.length === 0 ? (
                                <AsyncState compact kind="empty" title="No jobs reported" description="Jobs appear when the selected profile exposes execution history." />
                            ) : jobs.map((job) => (
                                <button
                                    key={job.id}
                                    type="button"
                                    onClick={() => setSelectedId(job.id)}
                                    className={`mb-1 w-full rounded-[var(--radius-control)] p-3 text-left transition-colors ${selectedId === job.id ? 'bg-primary/10 ring-1 ring-primary/20' : 'hover:bg-surface-subtle'}`}
                                >
                                    <div className="flex items-start justify-between gap-2">
                                        <div className="min-w-0"><p className="truncate text-sm font-medium text-content-primary">{job.title || job.id}</p><p className="mt-1 truncate font-mono text-[10px] text-content-muted">{job.id}</p></div>
                                        <StatusBadge status={job.state} />
                                    </div>
                                    <p className="mt-2 text-[10px] text-content-muted">{formatDate(job.created_at)}</p>
                                </button>
                            ))}
                        </div>
                    </Surface>

                    <div className="min-w-0 space-y-5">
                        <Surface className="p-5">
                            <div className="flex flex-wrap items-start justify-between gap-4">
                                <div className="min-w-0"><p className="text-[10px] font-bold uppercase tracking-widest text-content-muted">Selected job</p><h2 className="mt-1 truncate text-lg font-semibold">{detail?.title ?? selectedJob?.title ?? 'No job selected'}</h2><p className="mt-1 truncate font-mono text-xs text-content-muted">{selectedId ?? 'Select a job from the queue'}</p></div>
                                {detail && <StatusBadge status={detail.state} />}
                            </div>
                            {detail ? <>
                                <p className="mt-4 whitespace-pre-wrap text-sm text-content-muted">{detail.description || 'No description provided.'}</p>
                                <dl className="mt-4 grid gap-3 text-xs sm:grid-cols-2 lg:grid-cols-4"><div><dt className="text-content-muted">Backend</dt><dd className="mt-1 text-content-primary">{detail.execution_backend ?? 'Unknown'}</dd></div><div><dt className="text-content-muted">Runtime</dt><dd className="mt-1 text-content-primary">{detail.runtime_mode ?? detail.runtime_family ?? 'Unknown'}</dd></div><div><dt className="text-content-muted">Started</dt><dd className="mt-1 text-content-primary">{formatDate(detail.started_at)}</dd></div><div><dt className="text-content-muted">Elapsed</dt><dd className="mt-1 text-content-primary">{detail.elapsed_secs == null ? 'Unknown' : `${detail.elapsed_secs}s`}</dd></div></dl>
                                <div className="mt-5 flex flex-wrap gap-2">
                                    {canCancel && <Button size="sm" variant="danger" onClick={() => setCancelDialogOpen(true)} disabled={activeAction !== null}><Square className="size-3.5" aria-hidden="true" /> Cancel</Button>}
                                    {canRestart && <Button size="sm" variant="secondary" onClick={() => void handleAction('restart')} disabled={activeAction !== null}><RotateCcw className="size-3.5" aria-hidden="true" /> Restart</Button>}
                                    {canBrowseFiles && <Button size="sm" variant="secondary" onClick={() => void loadFiles(detail.id)}><Folder className="size-3.5" aria-hidden="true" /> Files</Button>}
                                    {!canCancel && !canRestart && !canBrowseFiles && <p className="text-xs text-content-muted">{unavailable.cancel ?? 'No additional actions are available for this job.'}</p>}
                                </div>
                            </> : <p className="mt-5 text-sm text-content-muted">Select a job to inspect its supported actions and recorded state.</p>}
                        </Surface>

                        <div className="grid gap-5 2xl:grid-cols-2">
                            <Surface className="p-5"><div className="flex items-center justify-between gap-2"><h2 className="text-sm font-semibold">Events</h2><Button size="icon" variant="ghost" aria-label="Refresh job events" onClick={() => selectedId && void loadDetail(selectedId)} disabled={!selectedId}><RefreshCw className="size-3.5" aria-hidden="true" /></Button></div><div className="mt-4 max-h-72 space-y-2 overflow-y-auto">{events.length === 0 ? <p className="text-xs text-content-muted">No events recorded.</p> : events.map((event, index) => <div key={event.id ?? index} className="rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle p-3"><div className="flex justify-between gap-2"><p className="text-xs font-medium">{event.event_type}</p><p className="text-[10px] text-content-muted">{formatDate(event.created_at)}</p></div>{event.data != null && <pre className="mt-2 max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[10px] text-content-muted">{JSON.stringify(event.data, null, 2)}</pre>}</div>)}</div></Surface>
                            <Surface className="p-5"><div className="flex items-center gap-2"><MessageSquarePlus className="size-4 text-primary" aria-hidden="true" /><h2 className="text-sm font-semibold">Prompt</h2></div>{canPrompt ? <><textarea aria-label="Follow-up prompt" value={prompt} onChange={(event) => setPrompt(event.currentTarget.value)} placeholder="Send a follow-up prompt" className="mt-4 min-h-24 w-full rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle p-3 text-sm text-content-primary outline-none focus-visible:ring-2 focus-visible:ring-primary/20" /><div className="mt-3 flex gap-2"><Button size="sm" variant="primary" onClick={() => void handleAction('prompt')} disabled={!prompt.trim() || activeAction !== null}>Send prompt</Button><Button size="sm" variant="secondary" onClick={() => void handleAction('done')} disabled={activeAction !== null}>Mark prompt complete</Button></div></> : <p className="mt-3 text-xs leading-relaxed text-content-muted">{unavailable.prompt ?? 'Interactive prompts are not supported for this job.'}</p>}</Surface>
                        </div>

                        <Surface className="p-5"><div className="flex items-center justify-between gap-2"><div className="flex items-center gap-2"><FileText className="size-4 text-primary" aria-hidden="true" /><h2 className="text-sm font-semibold">Files</h2></div><p className="max-w-56 truncate font-mono text-[10px] text-content-muted" title={filePath}>{filePath || '/'}</p></div>{canBrowseFiles ? <div className="mt-4 grid gap-4 lg:grid-cols-[minmax(14rem,20rem)_minmax(0,1fr)]"><div className="max-h-80 space-y-1 overflow-y-auto">{files.length === 0 ? <p className="text-xs text-content-muted">Select Files to list the supported job workspace.</p> : files.map((file) => <button key={file.path} type="button" onClick={() => file.is_dir ? void loadFiles(selectedId!, file.path) : void handleReadFile(file.path)} className="flex w-full items-center gap-2 rounded-[var(--radius-control)] px-2 py-1.5 text-left text-xs text-content-muted hover:bg-surface-subtle hover:text-content-primary"><Folder className="size-3 shrink-0" aria-hidden="true" /> <span className="truncate">{file.name}</span></button>)}</div><pre className="min-h-40 max-h-80 overflow-auto rounded-[var(--radius-control)] bg-surface-subtle p-3 whitespace-pre-wrap break-words font-mono text-[11px] text-content-muted">{fileContent || 'Select a file to inspect its content.'}</pre></div> : <p className="mt-3 text-xs text-content-muted">{unavailable.files ?? 'File browsing is not supported for this job.'}</p>}</Surface>

                        <Surface className="p-5"><h2 className="text-sm font-semibold">Transitions</h2><div className="mt-3 space-y-2">{detail?.transitions?.length ? detail.transitions.map((transition, index) => <div key={`${transition.timestamp}-${index}`} className="flex flex-wrap items-center justify-between gap-2 rounded-[var(--radius-control)] bg-surface-subtle p-3 text-xs"><span className="font-medium text-content-primary">{transition.from} → {transition.to}</span><span className="text-content-muted">{transition.reason ?? formatDate(transition.timestamp)}</span></div>) : <p className="text-xs text-content-muted">No transitions recorded.</p>}</div></Surface>
                    </div>
                </div>
            </div>
            <ConfirmDialog open={cancelDialogOpen} onOpenChange={setCancelDialogOpen} title="Cancel this job?" description={detail ? <>Job <span className="font-mono">{detail.title || detail.id}</span> will be asked to stop. The selected profile may still need time to report the resulting state.</> : 'The selected job will be asked to stop.'} confirmLabel="Cancel job" onConfirm={() => handleAction('cancel')} isConfirming={activeAction === 'cancel'} />
        </AgentPageShell>
    );
}
