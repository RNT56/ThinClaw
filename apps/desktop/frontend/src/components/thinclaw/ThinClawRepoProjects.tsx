import { useCallback, useEffect, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { motion } from 'framer-motion';
import type { LucideIcon } from 'lucide-react';
import {
    Activity,
    AlertTriangle,
    Bell,
    Bot,
    CheckCircle2,
    Circle,
    Container,
    Cpu,
    FileCheck2,
    FileText,
    GitBranch,
    FolderGit2,
    KeyRound,
    ListChecks,
    Pause,
    Play,
    Plus,
    RefreshCw,
    Settings2,
    ShieldCheck,
    Square,
    XCircle,
} from 'lucide-react';
import { toast } from 'sonner';
import { listen } from '@tauri-apps/api/event';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';
import { ThinClawRepoConnector } from './ThinClawRepoConnector';

const SHELL_PROJECTS: thinclaw.ThinClawRepoProject[] = [
    {
        id: 'shell-thinclaw-desktop',
        name: 'ThinClaw Desktop',
        repo_url: 'github.com/openclaw/thinclaw-desktop',
        default_branch: 'main',
        local_path: '~/Projects/thinclaw-desktop',
        description: 'Desktop app and runtime integration workstream.',
        state: 'setup_required',
        active_runs: 2,
        queued_items: 5,
        open_prs: 1,
        merge_gate_state: 'pending',
        github_app: 'pending',
        docker_agents: 'ready',
        credentials: 'partial',
        concurrency_limit: 3,
        auto_merge_policy: 'approved_only',
        notifications: 'enabled',
        updated_at: '2026-06-13T09:42:00Z',
        setup_checklist: [
            { key: 'feature_flag', label: 'Feature flag', state: 'pending', detail: 'Enable repo_projects.enabled before live dispatch' },
            { key: 'github_app', label: 'GitHub App', state: 'pending', detail: 'Installation awaiting repository grant' },
            { key: 'docker_agents', label: 'Docker coding agents', state: 'complete', detail: '3 runners registered' },
            { key: 'coding_backend', label: 'Coding backend', state: 'complete', detail: 'Default backend: worker' },
            { key: 'credentials', label: 'Credentials', state: 'pending', detail: 'CI token missing deploy scope' },
            { key: 'concurrency', label: 'Concurrency', state: 'complete', detail: 'Limit set to 3' },
            { key: 'auto_merge_policy', label: 'Auto-merge policy', state: 'complete', detail: 'Approved PRs with green checks' },
            { key: 'notifications', label: 'Notifications', state: 'complete', detail: 'Desktop and Slack enabled' },
        ],
        backlog: [
            { id: 'TC-112', title: 'Wire repo project command bus', priority: 'urgent', state: 'running', owner: 'worker-a', labels: ['frontend'] },
            { id: 'TC-109', title: 'Add merge gate policy preview', priority: 'high', state: 'queued', owner: 'worker-g', labels: ['ui'] },
            { id: 'TC-101', title: 'Persist worker run timeline events', priority: 'medium', state: 'blocked', owner: 'worker-c', labels: ['runtime'] },
            { id: 'TC-097', title: 'Normalize PR approval payloads', priority: 'medium', state: 'queued', owner: null, labels: ['github'] },
        ],
        worker_runs: [
            {
                id: 'run-2841',
                backlog_id: 'TC-112',
                agent: 'docker-agent-01',
                state: 'running',
                branch: 'workers/tc-112-command-bus',
                started_at: '2026-06-13T09:12:00Z',
                duration_secs: 1800,
                last_event: 'Awaiting type contract review',
            },
            {
                id: 'run-2837',
                backlog_id: 'TC-101',
                agent: 'docker-agent-02',
                state: 'paused',
                branch: 'workers/tc-101-timeline',
                started_at: '2026-06-13T08:20:00Z',
                duration_secs: 3540,
                last_event: 'Credential scope missing',
            },
        ],
        pull_requests: [
            { id: 'pr-88', title: 'Repo projects command skeleton', number: 88, branch: 'workers/tc-112-command-bus', state: 'draft', author: 'docker-agent-01' },
        ],
        ci_checks: [
            { id: 'ci-ts', name: 'frontend / typecheck', state: 'passed' },
            { id: 'ci-rust', name: 'backend / cargo test', state: 'running' },
            { id: 'ci-e2e', name: 'desktop / smoke', state: 'queued' },
        ],
        merge_gates: [
            { id: 'gate-github-app', label: 'GitHub App installation', state: 'pending', required: true, detail: 'Repository access not confirmed' },
            { id: 'gate-ci', label: 'Required CI checks', state: 'pending', required: true, detail: '1 running, 1 queued' },
            { id: 'gate-review', label: 'Human approval', state: 'blocked', required: true, detail: 'Review requested' },
        ],
    },
    {
        id: 'shell-runtime-contracts',
        name: 'Runtime Contracts',
        repo_url: 'github.com/openclaw/runtime-contracts',
        default_branch: 'main',
        description: 'Shared command contracts for desktop agents.',
        state: 'ready',
        active_runs: 0,
        queued_items: 2,
        open_prs: 0,
        merge_gate_state: 'passed',
        github_app: 'connected',
        docker_agents: 'ready',
        credentials: 'ready',
        concurrency_limit: 2,
        auto_merge_policy: 'manual',
        notifications: 'enabled',
        updated_at: '2026-06-13T08:15:00Z',
        setup_checklist: [
            { key: 'feature_flag', label: 'Feature flag', state: 'complete', detail: 'Supervisor enabled' },
            { key: 'github_app', label: 'GitHub App', state: 'complete', detail: 'Installed' },
            { key: 'docker_agents', label: 'Docker coding agents', state: 'complete', detail: '2 runners registered' },
            { key: 'coding_backend', label: 'Coding backend', state: 'complete', detail: 'Default backend: worker' },
            { key: 'credentials', label: 'Credentials', state: 'complete', detail: 'All scopes present' },
            { key: 'concurrency', label: 'Concurrency', state: 'complete', detail: 'Limit set to 2' },
            { key: 'auto_merge_policy', label: 'Auto-merge policy', state: 'pending', detail: 'Manual merge required' },
            { key: 'notifications', label: 'Notifications', state: 'complete', detail: 'Desktop enabled' },
        ],
        backlog: [
            { id: 'RC-031', title: 'Generate merge gate schema', priority: 'high', state: 'queued', owner: null, labels: ['schema'] },
            { id: 'RC-024', title: 'Document run cancellation payload', priority: 'low', state: 'queued', owner: null, labels: ['docs'] },
        ],
        worker_runs: [],
        pull_requests: [],
        ci_checks: [
            { id: 'ci-contracts', name: 'contracts / check', state: 'passed' },
            { id: 'ci-schema', name: 'schema / snapshot', state: 'passed' },
        ],
        merge_gates: [
            { id: 'gate-ci', label: 'Required CI checks', state: 'passed', required: true },
            { id: 'gate-policy', label: 'Merge policy', state: 'passed', required: true, detail: 'Manual merge' },
        ],
    },
];

const SHELL_EVENTS: thinclaw.ThinClawJobEvent[] = [
    {
        id: 'event-setup',
        event_type: 'repo.project.setup.pending',
        created_at: '2026-06-13T09:42:00Z',
        data: { gate: 'github_app', project: 'ThinClaw Desktop' },
    },
    {
        id: 'event-run',
        event_type: 'worker.run.started',
        created_at: '2026-06-13T09:12:00Z',
        data: { run_id: 'run-2841', agent: 'docker-agent-01' },
    },
    {
        id: 'event-ci',
        event_type: 'merge_gate.ci.running',
        created_at: '2026-06-13T09:39:00Z',
        data: { check: 'backend / cargo test' },
    },
];

function stateTone(state?: string) {
    switch ((state ?? '').toLowerCase()) {
        case 'complete':
        case 'completed':
        case 'connected':
        case 'done':
        case 'enabled':
        case 'passed':
        case 'ready':
        case 'merged':
            return 'text-emerald-400 bg-emerald-500/10 border-emerald-500/20';
        case 'blocked':
        case 'error':
        case 'failed':
        case 'missing':
            return 'text-red-400 bg-red-500/10 border-red-500/20';
        case 'paused':
        case 'partial':
        case 'pending':
        case 'queued':
        case 'setup_required':
            return 'text-amber-400 bg-amber-500/10 border-amber-500/20';
        case 'draft':
        case 'running':
            return 'text-blue-400 bg-blue-500/10 border-blue-500/20';
        default:
            return 'text-muted-foreground bg-white/[0.03] border-white/5';
    }
}

function setupIcon(key: string): LucideIcon {
    switch (key) {
        case 'feature_flag':
            return Settings2;
        case 'github_app':
            return FolderGit2;
        case 'docker_agents':
            return Container;
        case 'coding_backend':
            return Bot;
        case 'credentials':
            return KeyRound;
        case 'concurrency':
            return Cpu;
        case 'auto_merge_policy':
            return ShieldCheck;
        case 'notifications':
            return Bell;
        default:
            return Circle;
    }
}

function statusLabel(value?: string | null) {
    return (value || 'unknown').replace(/_/g, ' ');
}

function readinessIsReady(state?: string | null) {
    return ['complete', 'completed', 'connected', 'enabled', 'passed', 'ready'].includes((state ?? '').toLowerCase());
}

function projectCodingBackend(project: thinclaw.ThinClawRepoProject) {
    return project.coding_backend || 'worker';
}

function derivedReadinessItems(
    project: thinclaw.ThinClawRepoProject | null,
    isShellMode: boolean,
    unavailableReason: string | null,
): thinclaw.ThinClawRepoProjectSetupItem[] {
    if (!project) return [];

    const disabled = (unavailableReason ?? '').toLowerCase().includes('repo_projects.enabled=false')
        || (unavailableReason ?? '').toLowerCase().includes('repository projects are disabled');
    const checklist = new Map((project.setup_checklist ?? []).map((item) => [item.key, item]));
    const fallbackItems: thinclaw.ThinClawRepoProjectSetupItem[] = [
        {
            key: 'feature_flag',
            label: 'Feature flag',
            state: disabled ? 'blocked' : (isShellMode ? 'pending' : 'complete'),
            detail: disabled ? 'Set repo_projects.enabled=true.' : (isShellMode ? 'Waiting for live supervisor data.' : 'Supervisor commands available.'),
        },
        {
            key: 'github_app',
            label: 'GitHub App',
            state: project.github_app === 'connected' ? 'complete' : project.github_app,
            detail: project.github_app === 'connected' ? 'Repository access connected.' : 'Installation or token fallback needs attention.',
        },
        {
            key: 'docker_agents',
            label: 'Docker',
            state: project.docker_agents === 'ready' ? 'complete' : project.docker_agents,
            detail: project.docker_agents === 'ready' ? 'Coding workers can be dispatched.' : 'Worker runtime unavailable or degraded.',
        },
        {
            key: 'coding_backend',
            label: 'Coding backend',
            state: 'complete',
            detail: `Default backend: ${statusLabel(projectCodingBackend(project))}`,
        },
        {
            key: 'concurrency',
            label: 'Concurrency',
            state: project.concurrency_limit > 0 ? 'complete' : 'blocked',
            detail: `${project.concurrency_limit || 0} task${project.concurrency_limit === 1 ? '' : 's'} per project`,
        },
        {
            key: 'auto_merge_policy',
            label: 'Auto-merge',
            state: project.auto_merge_policy === 'disabled' ? 'blocked' : (project.auto_merge_policy === 'manual' ? 'pending' : 'complete'),
            detail: statusLabel(project.auto_merge_policy),
        },
    ];

    return fallbackItems.map((item) => ({ ...item, ...checklist.get(item.key) }));
}

function payloadLooksRepoProject(payload: unknown) {
    const serialized = JSON.stringify(payload ?? '').toLowerCase();
    return [
        'repo_project',
        'repo project',
        'repo_task',
        'repo_worker_run',
        'repo_merge_gate',
        'merge_gate',
        'worker.run',
    ].some((token) => serialized.includes(token));
}

function payloadProjectId(payload: any): string | null {
    return payload?.project_id
        ?? payload?.projectId
        ?? payload?.project?.id
        ?? payload?.data?.project_id
        ?? payload?.event?.project_id
        ?? null;
}

function formatDate(value?: string | null) {
    if (!value) return 'Not recorded';
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function formatDuration(seconds?: number | null) {
    if (seconds == null) return 'n/a';
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);
    if (hours > 0) return `${hours}h ${minutes % 60}m`;
    return `${minutes}m`;
}

function commandNotice(result: thinclaw.ThinClawRepoProjectCommandResponse, label: string) {
    if (result.ok) {
        toast.success(`${label} submitted`);
        return;
    }
    if (result.unavailable) {
        toast.info(`${label} is not wired yet`);
        return;
    }
    toast.error(result.message ?? `${label} failed`);
}

function StateBadge({ state, label }: { state?: string; label?: string }) {
    return (
        <span className={cn('inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider', stateTone(state))}>
            {label ?? statusLabel(state)}
        </span>
    );
}

function SectionCard({
    title,
    icon: Icon,
    children,
    action,
}: {
    title: string;
    icon: LucideIcon;
    children: ReactNode;
    action?: ReactNode;
}) {
    return (
        <div className="rounded-lg border border-border/40 bg-card/30 p-5">
            <div className="mb-4 flex items-center justify-between gap-3">
                <div className="flex items-center gap-2">
                    <Icon className="h-4 w-4 text-primary" />
                    <h3 className="text-sm font-bold">{title}</h3>
                </div>
                {action}
            </div>
            {children}
        </div>
    );
}

function MetricCard({ label, value, tone }: { label: string; value: string | number; tone?: string }) {
    return (
        <div className="rounded-lg border border-border/40 bg-card/30 p-4">
            <p className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground">{label}</p>
            <p className={cn('mt-1 text-2xl font-bold tabular-nums', tone)}>{value}</p>
        </div>
    );
}

export function ThinClawRepoProjects() {
    const [projects, setProjects] = useState<thinclaw.ThinClawRepoProject[]>(SHELL_PROJECTS);
    const [selectedProjectId, setSelectedProjectId] = useState<string | null>(SHELL_PROJECTS[0]?.id ?? null);
    const [detail, setDetail] = useState<thinclaw.ThinClawRepoProject | null>(SHELL_PROJECTS[0] ?? null);
    const [events, setEvents] = useState<thinclaw.ThinClawJobEvent[]>(SHELL_EVENTS);
    const [mergeGates, setMergeGates] = useState<thinclaw.ThinClawRepoMergeGate[]>(SHELL_PROJECTS[0]?.merge_gates ?? []);
    const [isShellMode, setIsShellMode] = useState(true);
    const [unavailableReason, setUnavailableReason] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(false);
    const [mutatingAction, setMutatingAction] = useState<string | null>(null);
    const [lastLiveRefreshAt, setLastLiveRefreshAt] = useState<string | null>(null);
    const [createInput, setCreateInput] = useState({
        name: '',
        repo_url: '',
        default_branch: 'main',
        local_path: '',
        description: '',
    });
    const [enqueueInput, setEnqueueInput] = useState({
        title: '',
        priority: 'medium',
        labels: '',
    });

    const selectedProject = useMemo(() => {
        if (detail?.id === selectedProjectId) return detail;
        return projects.find((project) => project.id === selectedProjectId) ?? null;
    }, [detail, projects, selectedProjectId]);

    const loadProjects = useCallback(async () => {
        setIsLoading(true);
        try {
            const response = await thinclaw.listRepoProjects();
            const nextProjects = response.projects.length > 0 ? response.projects : SHELL_PROJECTS;
            setProjects(nextProjects);
            setIsShellMode(response.projects.length === 0);
            setUnavailableReason(response.unavailable?.reason ?? null);
            setSelectedProjectId((current) => (
                current && nextProjects.some((project) => project.id === current)
                    ? current
                    : nextProjects[0]?.id ?? null
            ));
        } finally {
            setIsLoading(false);
        }
    }, []);

    const loadSelectedProject = useCallback(async (projectId: string) => {
        const fallbackProject = projects.find((project) => project.id === projectId) ?? null;
        const [projectResponse, eventResponse, gateResponse] = await Promise.all([
            thinclaw.getRepoProject(projectId),
            thinclaw.getRepoProjectEvents(projectId, 40),
            thinclaw.getRepoProjectMergeGates(projectId),
        ]);
        const nextProject = projectResponse.project ?? fallbackProject;
        setDetail(nextProject);
        setEvents(eventResponse.events.length > 0 ? eventResponse.events : (isShellMode ? SHELL_EVENTS : []));
        setMergeGates(gateResponse.gates.length > 0 ? gateResponse.gates : (nextProject?.merge_gates ?? []));
        setUnavailableReason(
            projectResponse.unavailable?.reason
            ?? eventResponse.unavailable?.reason
            ?? gateResponse.unavailable?.reason
            ?? null,
        );
    }, [isShellMode, projects]);

    useEffect(() => {
        loadProjects();
    }, [loadProjects]);

    useEffect(() => {
        if (!selectedProjectId) {
            setDetail(null);
            setEvents([]);
            setMergeGates([]);
            return;
        }
        loadSelectedProject(selectedProjectId);
    }, [loadSelectedProject, selectedProjectId]);

    useEffect(() => {
        const unlistenPromise = listen<any>('thinclaw-event', (event) => {
            const payload = event.payload;
            if (!payloadLooksRepoProject(payload)) return;

            const projectId = payloadProjectId(payload);
            setLastLiveRefreshAt(new Date().toISOString());
            loadProjects();
            if (selectedProjectId && (!projectId || projectId === selectedProjectId)) {
                loadSelectedProject(selectedProjectId);
            }
        });

        return () => {
            unlistenPromise.then((unlisten) => unlisten()).catch(() => { });
        };
    }, [loadProjects, loadSelectedProject, selectedProjectId]);

    const stats = useMemo(() => {
        const activeRuns = projects.reduce((sum, project) => sum + project.active_runs, 0);
        const queuedItems = projects.reduce((sum, project) => sum + project.queued_items, 0);
        const openPrs = projects.reduce((sum, project) => sum + project.open_prs, 0);
        const blockedGates = projects.filter((project) => ['blocked', 'failed', 'pending'].includes(project.merge_gate_state)).length;
        return { activeRuns, queuedItems, openPrs, blockedGates };
    }, [projects]);

    const pendingGate = mergeGates.find((gate) => gate.state === 'pending' || gate.state === 'blocked') ?? null;
    const cancellableRun = selectedProject?.worker_runs?.find((run) => ['queued', 'running', 'paused'].includes(run.state)) ?? selectedProject?.worker_runs?.[0] ?? null;
    const readinessItems = useMemo(
        () => derivedReadinessItems(selectedProject, isShellMode, unavailableReason),
        [isShellMode, selectedProject, unavailableReason],
    );
    const readinessScore = readinessItems.length === 0
        ? 0
        : Math.round((readinessItems.filter((item) => readinessIsReady(item.state)).length / readinessItems.length) * 100);

    const runCommand = async (
        label: string,
        action: () => Promise<thinclaw.ThinClawRepoProjectCommandResponse>,
    ) => {
        setMutatingAction(label);
        try {
            const result = await action();
            commandNotice(result, label);
            if (result.project) {
                setDetail(result.project);
                setSelectedProjectId(result.project.id);
            }
            if (result.ok) {
                const projectId = result.project?.id ?? selectedProjectId;
                await loadProjects();
                if (projectId) await loadSelectedProject(projectId);
            }
            return result;
        } finally {
            setMutatingAction(null);
        }
    };

    const createProject = async () => {
        const name = createInput.name.trim();
        const repoUrl = createInput.repo_url.trim();
        if (!name || !repoUrl) {
            toast.error('Project name and repo URL are required');
            return;
        }
        const result = await runCommand('Create project', () => thinclaw.createRepoProject({
            name,
            repo_url: repoUrl,
            default_branch: createInput.default_branch.trim() || 'main',
            local_path: createInput.local_path.trim() || null,
            description: createInput.description.trim() || null,
        }));
        if (result?.ok) {
            setCreateInput({ name: '', repo_url: '', default_branch: 'main', local_path: '', description: '' });
        }
    };

    const enqueueWork = async () => {
        if (!selectedProject) return;
        const title = enqueueInput.title.trim();
        if (!title) {
            toast.error('Backlog title is required');
            return;
        }
        const result = await runCommand('Enqueue work', () => thinclaw.enqueueRepoProject(selectedProject.id, {
            title,
            priority: enqueueInput.priority,
            labels: enqueueInput.labels.split(',').map((label) => label.trim()).filter(Boolean),
        }));
        if (result?.ok) {
            setEnqueueInput({ title: '', priority: 'medium', labels: '' });
        }
    };

    const approveGate = () => {
        if (!selectedProject) return;
        runCommand('Approve gate', () => thinclaw.approveRepoProject(selectedProject.id, {
            approval_id: pendingGate?.id ?? 'repo-project-shell-approval',
            decision: 'approve',
            note: 'Approved from ThinClaw Desktop',
        }));
    };

    const actionDisabled = !selectedProject || Boolean(mutatingAction);
    const backlog = selectedProject?.backlog ?? [];
    const runs = selectedProject?.worker_runs ?? [];
    const activeWorkers = runs.filter((run) => ['queued', 'running', 'paused'].includes(run.state));
    const pullRequests = selectedProject?.pull_requests ?? [];
    const checks = selectedProject?.ci_checks ?? [];
    const canCreateProject = createInput.name.trim().length > 0 && createInput.repo_url.trim().length > 0 && !mutatingAction;
    const canEnqueueWork = Boolean(selectedProject) && enqueueInput.title.trim().length > 0 && !mutatingAction;

    return (
        <motion.div className="flex-1 overflow-y-auto p-8 space-y-6" initial={{ opacity: 0 }} animate={{ opacity: 1 }}>
            <div className="flex flex-wrap items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                    <div className="rounded-lg border border-blue-500/20 bg-blue-500/10 p-2.5">
                        <GitBranch className="h-5 w-5 text-primary" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Repo Projects</h1>
                        <p className="text-xs text-muted-foreground">Repository work queues, worker runs, pull requests, CI, and merge gates</p>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    {lastLiveRefreshAt && (
                        <span className="hidden text-[10px] text-muted-foreground md:inline">
                            Live {formatDate(lastLiveRefreshAt)}
                        </span>
                    )}
                    <StateBadge state={isShellMode ? 'setup_required' : 'ready'} label={isShellMode ? 'shell' : 'live'} />
                    <button
                        onClick={loadProjects}
                        className="rounded-lg border border-white/5 bg-white/[0.03] p-2 text-muted-foreground transition-colors hover:bg-white/5 hover:text-foreground"
                        title="Refresh repo projects"
                    >
                        <RefreshCw className={cn('h-4 w-4', isLoading && 'animate-spin')} />
                    </button>
                </div>
            </div>

            {(isShellMode || unavailableReason) && (
                <div className="flex items-start gap-3 rounded-lg border border-amber-500/20 bg-amber-500/10 p-4 text-sm text-amber-200">
                    <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
                    <span>
                        {unavailableReason
                            ? `Repo project commands are unavailable: ${unavailableReason}`
                            : 'Showing repo projects shell data.'}
                    </span>
                </div>
            )}

            <ThinClawRepoConnector onConnected={loadProjects} />

            <div className="grid grid-cols-2 gap-3 lg:grid-cols-5">
                <MetricCard label="Projects" value={projects.length} />
                <MetricCard label="Active Runs" value={stats.activeRuns} tone={stats.activeRuns > 0 ? 'text-blue-400' : undefined} />
                <MetricCard label="Queued" value={stats.queuedItems} />
                <MetricCard label="Open PRs" value={stats.openPrs} />
                <MetricCard label="Gate Issues" value={stats.blockedGates} tone={stats.blockedGates > 0 ? 'text-amber-400' : 'text-emerald-400'} />
            </div>

            <div className="grid grid-cols-1 gap-6 xl:grid-cols-[360px_1fr]">
                <div className="space-y-4">
                    <div className="rounded-lg border border-border/40 bg-card/30 overflow-hidden">
                        <div className="flex items-center justify-between border-b border-border/40 px-4 py-3">
                            <p className="text-xs font-bold uppercase tracking-widest text-muted-foreground">Projects</p>
                            <StateBadge state={isShellMode ? 'setup_required' : 'ready'} label={isShellMode ? 'shell' : 'live'} />
                        </div>
                        <div className="max-h-[560px] overflow-y-auto">
                            {projects.map((project) => (
                                <button
                                    key={project.id}
                                    onClick={() => setSelectedProjectId(project.id)}
                                    className={cn(
                                        'w-full border-b border-border/30 px-4 py-3 text-left transition-colors hover:bg-white/[0.03]',
                                        selectedProjectId === project.id && 'bg-primary/10',
                                    )}
                                >
                                    <div className="flex items-start justify-between gap-3">
                                        <div className="min-w-0">
                                            <p className="truncate text-sm font-semibold">{project.name}</p>
                                            <p className="mt-1 truncate text-[10px] font-mono text-muted-foreground">{project.repo_url}</p>
                                        </div>
                                        <StateBadge state={project.state} />
                                    </div>
                                    <div className="mt-3 grid grid-cols-3 gap-2 text-[10px] text-muted-foreground">
                                        <span>{project.active_runs} runs</span>
                                        <span>{project.queued_items} queued</span>
                                        <span>{project.open_prs} PRs</span>
                                    </div>
                                </button>
                            ))}
                        </div>
                    </div>

                    <SectionCard title="Create Project" icon={Plus}>
                        <div className="space-y-3">
                            <input
                                value={createInput.name}
                                onChange={(event) => setCreateInput((current) => ({ ...current, name: event.target.value }))}
                                placeholder="Project name"
                                className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-none placeholder:text-muted-foreground focus:border-primary/40"
                            />
                            <input
                                value={createInput.repo_url}
                                onChange={(event) => setCreateInput((current) => ({ ...current, repo_url: event.target.value }))}
                                placeholder="github.com/owner/repo"
                                className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs font-mono outline-none placeholder:text-muted-foreground focus:border-primary/40"
                            />
                            <div className="grid grid-cols-2 gap-2">
                                <input
                                    value={createInput.default_branch}
                                    onChange={(event) => setCreateInput((current) => ({ ...current, default_branch: event.target.value }))}
                                    placeholder="Branch"
                                    className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-none placeholder:text-muted-foreground focus:border-primary/40"
                                />
                                <input
                                    value={createInput.local_path}
                                    onChange={(event) => setCreateInput((current) => ({ ...current, local_path: event.target.value }))}
                                    placeholder="Local path"
                                    className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-none placeholder:text-muted-foreground focus:border-primary/40"
                                />
                            </div>
                            <textarea
                                value={createInput.description}
                                onChange={(event) => setCreateInput((current) => ({ ...current, description: event.target.value }))}
                                placeholder="Description"
                                rows={2}
                                className="w-full resize-none rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-none placeholder:text-muted-foreground focus:border-primary/40"
                            />
                            <button
                                onClick={createProject}
                                disabled={!canCreateProject}
                                className="flex w-full items-center justify-center gap-1.5 rounded-lg border border-primary/20 bg-primary/10 px-3 py-2 text-xs text-primary transition-colors hover:bg-primary/20 disabled:opacity-40"
                            >
                                <Plus className="h-3.5 w-3.5" />
                                Create
                            </button>
                        </div>
                    </SectionCard>
                </div>

                <div className="space-y-6">
                    <SectionCard
                        title="Project Detail"
                        icon={FileText}
                        action={selectedProject ? <StateBadge state={selectedProject.state} /> : undefined}
                    >
                        {selectedProject ? (
                            <>
                                <div className="flex flex-wrap items-start justify-between gap-4">
                                    <div className="min-w-0">
                                        <h2 className="truncate text-lg font-bold">{selectedProject.name}</h2>
                                        <p className="mt-1 max-w-3xl text-sm text-muted-foreground">{selectedProject.description ?? 'No project description recorded.'}</p>
                                    </div>
                                    <div className="flex flex-wrap gap-2">
                                        <button
                                            disabled={actionDisabled}
                                            onClick={() => runCommand('Start project', () => thinclaw.startRepoProject(selectedProject.id))}
                                            className="flex items-center gap-1.5 rounded-lg border border-primary/20 bg-primary/10 px-3 py-1.5 text-xs text-primary hover:bg-primary/20 disabled:opacity-40"
                                        >
                                            <Play className="h-3.5 w-3.5" />
                                            Start
                                        </button>
                                        <button
                                            disabled={actionDisabled}
                                            onClick={() => runCommand('Plan project', () => thinclaw.planRepoProject(selectedProject.id))}
                                            className="flex items-center gap-1.5 rounded-lg border border-blue-500/20 bg-blue-500/10 px-3 py-1.5 text-xs text-blue-300 hover:bg-blue-500/20 disabled:opacity-40"
                                        >
                                            <ListChecks className="h-3.5 w-3.5" />
                                            Plan
                                        </button>
                                        <button
                                            disabled={actionDisabled}
                                            onClick={() => runCommand('Pause project', () => thinclaw.pauseRepoProject(selectedProject.id))}
                                            className="flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.03] px-3 py-1.5 text-xs hover:bg-white/5 disabled:opacity-40"
                                        >
                                            <Pause className="h-3.5 w-3.5" />
                                            Pause
                                        </button>
                                        <button
                                            disabled={actionDisabled}
                                            onClick={() => runCommand('Resume project', () => thinclaw.resumeRepoProject(selectedProject.id))}
                                            className="flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.03] px-3 py-1.5 text-xs hover:bg-white/5 disabled:opacity-40"
                                        >
                                            <Play className="h-3.5 w-3.5" />
                                            Resume
                                        </button>
                                        <button
                                            disabled={actionDisabled}
                                            onClick={() => runCommand('Cancel project', () => thinclaw.cancelRepoProject(selectedProject.id, cancellableRun?.id))}
                                            className="flex items-center gap-1.5 rounded-lg border border-red-500/20 bg-red-500/10 px-3 py-1.5 text-xs text-red-300 hover:bg-red-500/20 disabled:opacity-40"
                                        >
                                            <Square className="h-3.5 w-3.5" />
                                            Cancel
                                        </button>
                                    </div>
                                </div>

                                <div className="mt-5 grid grid-cols-2 gap-3 lg:grid-cols-6">
                                    <div>
                                        <p className="text-[10px] font-bold uppercase text-muted-foreground">Default branch</p>
                                        <p className="mt-1 text-xs font-mono">{selectedProject.default_branch}</p>
                                    </div>
                                    <div>
                                        <p className="text-[10px] font-bold uppercase text-muted-foreground">Local path</p>
                                        <p className="mt-1 truncate text-xs font-mono">{selectedProject.local_path ?? 'not mounted'}</p>
                                    </div>
                                    <div>
                                        <p className="text-[10px] font-bold uppercase text-muted-foreground">Concurrency</p>
                                        <p className="mt-1 text-xs">{selectedProject.concurrency_limit} agents</p>
                                    </div>
                                    <div>
                                        <p className="text-[10px] font-bold uppercase text-muted-foreground">Backend</p>
                                        <p className="mt-1 text-xs">{statusLabel(projectCodingBackend(selectedProject))}</p>
                                    </div>
                                    <div>
                                        <p className="text-[10px] font-bold uppercase text-muted-foreground">Auto-merge</p>
                                        <p className="mt-1 text-xs">{statusLabel(selectedProject.auto_merge_policy)}</p>
                                    </div>
                                    <div>
                                        <p className="text-[10px] font-bold uppercase text-muted-foreground">Updated</p>
                                        <p className="mt-1 text-xs">{formatDate(selectedProject.updated_at)}</p>
                                    </div>
                                </div>
                            </>
                        ) : (
                            <p className="text-sm text-muted-foreground">No project selected.</p>
                        )}
                    </SectionCard>

                    <div className="grid grid-cols-1 gap-6 2xl:grid-cols-2">
                        <SectionCard
                            title="Setup Readiness"
                            icon={ListChecks}
                            action={<StateBadge state={readinessScore === 100 ? 'ready' : 'pending'} label={`${readinessScore}%`} />}
                        >
                            <div className="space-y-2">
                                {readinessItems.map((item) => {
                                    const Icon = setupIcon(item.key);
                                    const ok = readinessIsReady(item.state);
                                    return (
                                        <div key={item.key} className="flex items-start gap-3 border-b border-white/5 pb-2 last:border-0 last:pb-0">
                                            <div className="mt-0.5 rounded-md bg-white/[0.03] p-1.5">
                                                <Icon className="h-3.5 w-3.5 text-primary" />
                                            </div>
                                            <div className="min-w-0 flex-1">
                                                <div className="flex items-center justify-between gap-2">
                                                    <p className="text-sm font-semibold">{item.label}</p>
                                                    {ok ? <CheckCircle2 className="h-4 w-4 text-emerald-400" /> : <StateBadge state={item.state} />}
                                                </div>
                                                {item.detail && <p className="mt-1 text-xs text-muted-foreground">{item.detail}</p>}
                                            </div>
                                        </div>
                                    );
                                })}
                            </div>
                        </SectionCard>

                        <SectionCard
                            title="Active Workers"
                            icon={Bot}
                            action={<StateBadge state={activeWorkers.length > 0 ? 'running' : 'ready'} label={`${activeWorkers.length} active`} />}
                        >
                            <div className="space-y-2">
                                {activeWorkers.length === 0 ? (
                                    <p className="text-xs text-muted-foreground">No active worker runs.</p>
                                ) : activeWorkers.map((run) => (
                                    <div key={run.id} className="border-b border-white/5 pb-3 last:border-0 last:pb-0">
                                        <div className="flex items-start justify-between gap-3">
                                            <div className="min-w-0">
                                                <p className="truncate text-sm font-semibold">{run.agent}</p>
                                                <p className="mt-1 truncate text-[10px] font-mono text-muted-foreground">{run.branch ?? run.id}</p>
                                            </div>
                                            <StateBadge state={run.state} />
                                        </div>
                                        <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-[10px] text-muted-foreground">
                                            <span>{run.backlog_id ?? 'unassigned'}</span>
                                            <span>{formatDuration(run.duration_secs)}</span>
                                            <span>{formatDate(run.started_at)}</span>
                                        </div>
                                        {run.last_event && <p className="mt-2 text-xs text-muted-foreground">{run.last_event}</p>}
                                    </div>
                                ))}
                            </div>
                        </SectionCard>
                    </div>

                    <SectionCard
                        title="Backlog"
                        icon={Activity}
                        action={<button onClick={enqueueWork} disabled={!canEnqueueWork} className="flex items-center gap-1.5 rounded-lg border border-primary/20 bg-primary/10 px-2.5 py-1 text-xs text-primary hover:bg-primary/20 disabled:opacity-40"><Plus className="h-3.5 w-3.5" /> Enqueue</button>}
                    >
                        <div className="mb-4 grid grid-cols-1 gap-2 lg:grid-cols-[1fr_130px_180px]">
                            <input
                                value={enqueueInput.title}
                                onChange={(event) => setEnqueueInput((current) => ({ ...current, title: event.target.value }))}
                                placeholder="Backlog item"
                                disabled={!selectedProject}
                                className="rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-none placeholder:text-muted-foreground focus:border-primary/40 disabled:opacity-40"
                            />
                            <select
                                value={enqueueInput.priority}
                                onChange={(event) => setEnqueueInput((current) => ({ ...current, priority: event.target.value }))}
                                disabled={!selectedProject}
                                className="rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-none focus:border-primary/40 disabled:opacity-40"
                            >
                                <option value="low">Low</option>
                                <option value="medium">Medium</option>
                                <option value="high">High</option>
                                <option value="urgent">Urgent</option>
                            </select>
                            <input
                                value={enqueueInput.labels}
                                onChange={(event) => setEnqueueInput((current) => ({ ...current, labels: event.target.value }))}
                                placeholder="labels"
                                disabled={!selectedProject}
                                className="rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-none placeholder:text-muted-foreground focus:border-primary/40 disabled:opacity-40"
                            />
                        </div>
                        <div className="overflow-x-auto">
                            <table className="w-full min-w-[640px] text-left text-xs">
                                <thead className="text-[10px] uppercase tracking-widest text-muted-foreground">
                                    <tr className="border-b border-white/5">
                                        <th className="py-2 font-bold">Key</th>
                                        <th className="py-2 font-bold">Item</th>
                                        <th className="py-2 font-bold">Priority</th>
                                        <th className="py-2 font-bold">State</th>
                                        <th className="py-2 font-bold">Owner</th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {backlog.length === 0 ? (
                                        <tr>
                                            <td colSpan={5} className="py-6 text-center text-muted-foreground">No backlog items recorded.</td>
                                        </tr>
                                    ) : backlog.map((item) => (
                                        <tr key={item.id} className="border-b border-white/5 last:border-0">
                                            <td className="py-3 font-mono text-muted-foreground">{item.id}</td>
                                            <td className="py-3">
                                                <p className="font-semibold">{item.title}</p>
                                                {(item.labels ?? []).length > 0 && (
                                                    <p className="mt-1 text-[10px] text-muted-foreground">{item.labels!.join(', ')}</p>
                                                )}
                                            </td>
                                            <td className="py-3"><StateBadge state={item.priority} /></td>
                                            <td className="py-3"><StateBadge state={item.state} /></td>
                                            <td className="py-3 text-muted-foreground">{item.owner ?? 'unassigned'}</td>
                                        </tr>
                                    ))}
                                </tbody>
                            </table>
                        </div>
                    </SectionCard>

                    <div className="grid grid-cols-1 gap-6 2xl:grid-cols-3">
                        <SectionCard title="Pull Requests" icon={FileText}>
                            <div className="space-y-3">
                                {pullRequests.length === 0 ? (
                                    <p className="text-xs text-muted-foreground">No pull requests linked.</p>
                                ) : pullRequests.map((pr) => (
                                    <div key={pr.id} className="border-b border-white/5 pb-3 last:border-0 last:pb-0">
                                        <div className="flex items-start justify-between gap-2">
                                            <p className="text-sm font-semibold">{pr.number ? `#${pr.number} ` : ''}{pr.title}</p>
                                            <StateBadge state={pr.state} />
                                        </div>
                                        <p className="mt-1 truncate text-[10px] font-mono text-muted-foreground">{pr.branch ?? pr.url ?? pr.id}</p>
                                    </div>
                                ))}
                            </div>
                        </SectionCard>

                        <SectionCard title="CI" icon={FileCheck2}>
                            <div className="space-y-2">
                                {checks.length === 0 ? (
                                    <p className="text-xs text-muted-foreground">No CI checks recorded.</p>
                                ) : checks.map((check) => (
                                    <div key={check.id} className="flex items-center justify-between gap-3 border-b border-white/5 py-2 first:pt-0 last:border-0 last:pb-0">
                                        <p className="truncate text-sm">{check.name}</p>
                                        <StateBadge state={check.state} />
                                    </div>
                                ))}
                            </div>
                        </SectionCard>

                        <SectionCard
                            title="Merge Gates"
                            icon={ShieldCheck}
                            action={<button onClick={approveGate} disabled={actionDisabled || !pendingGate} className="flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/[0.03] px-2.5 py-1 text-xs hover:bg-white/5 disabled:opacity-40"><ShieldCheck className="h-3.5 w-3.5" /> Approve</button>}
                        >
                            <div className="space-y-2">
                                {mergeGates.length === 0 ? (
                                    <p className="text-xs text-muted-foreground">No merge gates configured.</p>
                                ) : mergeGates.map((gate) => (
                                    <div key={gate.id} className="border-b border-white/5 pb-3 last:border-0 last:pb-0">
                                        <div className="flex items-center justify-between gap-3">
                                            <div className="flex min-w-0 items-center gap-2">
                                                {gate.state === 'passed' ? <CheckCircle2 className="h-4 w-4 text-emerald-400" /> : gate.state === 'failed' ? <XCircle className="h-4 w-4 text-red-400" /> : <Circle className="h-4 w-4 text-amber-400" />}
                                                <p className="truncate text-sm font-semibold">{gate.label}</p>
                                            </div>
                                            <StateBadge state={gate.state} />
                                        </div>
                                        {gate.detail && <p className="mt-1 text-xs text-muted-foreground">{gate.detail}</p>}
                                    </div>
                                ))}
                            </div>
                        </SectionCard>
                    </div>

                    <SectionCard title="Event Timeline" icon={Settings2}>
                        <div className="max-h-72 space-y-2 overflow-y-auto">
                            {events.length === 0 ? (
                                <p className="text-xs text-muted-foreground">No project events recorded.</p>
                            ) : events.map((event, index) => (
                                <div key={event.id ?? `${event.event_type}-${index}`} className="rounded-md border border-white/5 bg-black/20 p-3">
                                    <div className="flex items-center justify-between gap-3">
                                        <p className="text-xs font-semibold">{event.event_type}</p>
                                        <p className="text-[10px] text-muted-foreground">{formatDate(event.created_at)}</p>
                                    </div>
                                    {event.data != null && (
                                        <pre className="mt-2 overflow-x-auto text-[10px] text-muted-foreground">{JSON.stringify(event.data, null, 2)}</pre>
                                    )}
                                </div>
                            ))}
                        </div>
                    </SectionCard>
                </div>
            </div>
        </motion.div>
    );
}
