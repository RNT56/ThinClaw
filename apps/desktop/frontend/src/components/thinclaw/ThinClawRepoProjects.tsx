import { motion } from 'framer-motion';
import {
    Activity,
    AlertTriangle,
    Bot,
    CheckCircle2,
    Circle,
    FileCheck2,
    FileText,
    GitBranch,
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
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';
import { ThinClawRepoConnector } from './ThinClawRepoConnector';
import { MetricCard, SectionCard, StateBadge } from './repo-projects/panels';
import { useRepoProjects } from './repo-projects/use-repo-projects';
import { formatDate, formatDuration, projectCodingBackend, readinessIsReady, setupIcon, statusLabel } from './repo-projects/utils';

export function ThinClawRepoProjects() {
    const {
        projects, selectedProjectId, setSelectedProjectId, events, mergeGates,
        isShellMode, unavailableReason, isLoading, lastLiveRefreshAt, createInput,
        setCreateInput, enqueueInput, setEnqueueInput, selectedProject, loadProjects,
        stats, pendingGate, cancellableRun, readinessItems, readinessScore, runCommand,
        createProject, enqueueWork, approveGate, actionDisabled, backlog, activeWorkers,
        pullRequests, checks, canCreateProject, canEnqueueWork,
    } = useRepoProjects();

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
                        className="rounded-lg border border-white/5 bg-white/3 p-2 text-muted-foreground transition-colors hover:bg-white/5 hover:text-foreground"
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
                                        'w-full border-b border-border/30 px-4 py-3 text-left transition-colors hover:bg-white/3',
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
                                className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                            />
                            <input
                                value={createInput.repo_url}
                                onChange={(event) => setCreateInput((current) => ({ ...current, repo_url: event.target.value }))}
                                placeholder="github.com/owner/repo"
                                className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs font-mono outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                            />
                            <div className="grid grid-cols-2 gap-2">
                                <input
                                    value={createInput.default_branch}
                                    onChange={(event) => setCreateInput((current) => ({ ...current, default_branch: event.target.value }))}
                                    placeholder="Branch"
                                    className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                                />
                                <input
                                    value={createInput.local_path}
                                    onChange={(event) => setCreateInput((current) => ({ ...current, local_path: event.target.value }))}
                                    placeholder="Local path"
                                    className="w-full rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
                                />
                            </div>
                            <textarea
                                value={createInput.description}
                                onChange={(event) => setCreateInput((current) => ({ ...current, description: event.target.value }))}
                                placeholder="Description"
                                rows={2}
                                className="w-full resize-none rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40"
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
                                            className="flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/3 px-3 py-1.5 text-xs hover:bg-white/5 disabled:opacity-40"
                                        >
                                            <Pause className="h-3.5 w-3.5" />
                                            Pause
                                        </button>
                                        <button
                                            disabled={actionDisabled}
                                            onClick={() => runCommand('Resume project', () => thinclaw.resumeRepoProject(selectedProject.id))}
                                            className="flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/3 px-3 py-1.5 text-xs hover:bg-white/5 disabled:opacity-40"
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
                                        <p className="text-[10px] font-bold uppercase text-muted-foreground">Write mode</p>
                                        <p className="mt-1 text-xs">{statusLabel(selectedProject.write_mode)}</p>
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
                                            <div className="mt-0.5 rounded-md bg-white/3 p-1.5">
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
                                className="rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40 disabled:opacity-40"
                            />
                            <select
                                value={enqueueInput.priority}
                                onChange={(event) => setEnqueueInput((current) => ({ ...current, priority: event.target.value }))}
                                disabled={!selectedProject}
                                className="rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden focus:border-primary/40 disabled:opacity-40"
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
                                className="rounded-lg border border-white/5 bg-black/20 px-3 py-2 text-xs outline-hidden placeholder:text-muted-foreground focus:border-primary/40 disabled:opacity-40"
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
                            action={<button onClick={approveGate} disabled={actionDisabled || !pendingGate} className="flex items-center gap-1.5 rounded-lg border border-white/5 bg-white/3 px-2.5 py-1 text-xs hover:bg-white/5 disabled:opacity-40"><ShieldCheck className="h-3.5 w-3.5" /> Approve</button>}
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
