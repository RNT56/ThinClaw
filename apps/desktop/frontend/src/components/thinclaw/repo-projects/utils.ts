import type { LucideIcon } from 'lucide-react';
import { Bell, Bot, Circle, Container, Cpu, FolderGit2, KeyRound, Settings2, ShieldCheck } from 'lucide-react';
import { toast } from 'sonner';

import type * as thinclaw from '../../../lib/thinclaw';

export function stateTone(state?: string) {
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
            return 'text-muted-foreground bg-white/3 border-white/5';
    }
}
export function setupIcon(key: string): LucideIcon {
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

export function statusLabel(value?: string | null) {
    return (value || 'unknown').replace(/_/g, ' ');
}

export function readinessIsReady(state?: string | null) {
    return ['complete', 'completed', 'connected', 'enabled', 'passed', 'ready'].includes((state ?? '').toLowerCase());
}

export function projectCodingBackend(project: thinclaw.ThinClawRepoProject) {
    return project.coding_backend || 'worker';
}

export function derivedReadinessItems(
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
            key: 'write_mode',
            label: 'Write mode',
            state: 'complete',
            detail: statusLabel(project.write_mode),
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

export function payloadLooksRepoProject(payload: unknown) {
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

export function payloadProjectId(payload: any): string | null {
    return payload?.project_id
        ?? payload?.projectId
        ?? payload?.project?.id
        ?? payload?.data?.project_id
        ?? payload?.event?.project_id
        ?? null;
}

export function formatDate(value?: string | null) {
    if (!value) return 'Not recorded';
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

export function formatDuration(seconds?: number | null) {
    if (seconds == null) return 'n/a';
    const minutes = Math.floor(seconds / 60);
    const hours = Math.floor(minutes / 60);
    if (hours > 0) return `${hours}h ${minutes % 60}m`;
    return `${minutes}m`;
}

export function commandNotice(result: thinclaw.ThinClawRepoProjectCommandResponse, label: string) {
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
