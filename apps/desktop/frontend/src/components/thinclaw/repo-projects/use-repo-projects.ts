import { useCallback, useEffect, useMemo, useState } from 'react';
import { toast } from 'sonner';

import * as thinclaw from '../../../lib/thinclaw';
import { useThinClawEvents } from '../../../hooks/use-thinclaw-stream';
import { SHELL_EVENTS, SHELL_PROJECTS } from './fixtures';
import {
    commandNotice, derivedReadinessItems, payloadLooksRepoProject, payloadProjectId,
    readinessIsReady,
} from './utils';

export function useRepoProjects() {
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

    useThinClawEvents((payload) => {
        if (!payloadLooksRepoProject(payload)) return;

        const projectId = payloadProjectId(payload);
        setLastLiveRefreshAt(new Date().toISOString());
        loadProjects();
        if (selectedProjectId && (!projectId || projectId === selectedProjectId)) {
            loadSelectedProject(selectedProjectId);
        }
    });

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

    return {
        projects, selectedProjectId, setSelectedProjectId, events, mergeGates, isShellMode,
        unavailableReason, isLoading, mutatingAction, lastLiveRefreshAt, createInput,
        setCreateInput, enqueueInput, setEnqueueInput, selectedProject, loadProjects, stats,
        pendingGate, cancellableRun, readinessItems, readinessScore, runCommand, createProject,
        enqueueWork, approveGate, actionDisabled, backlog, activeWorkers, pullRequests, checks,
        canCreateProject, canEnqueueWork,
    };
}
