import { useCallback, useEffect, useMemo, useState } from 'react';
import { Activity, ArrowRight, CircleDollarSign, Clock3, Radio, RefreshCw, ShieldAlert, Timer } from 'lucide-react';

import * as thinclaw from '../../lib/thinclaw';
import { useChatLayout } from '../chat/ChatProvider';
import { AgentPageShell, Button, MetricCard, Notice, StatusBadge, Surface } from '../ui';
import { useAgentCockpit } from './AgentCockpitProvider';

interface HomeEvidence {
    sessions: number | null;
    jobs: number | null;
    runningJobs: number | null;
    automations: number | null;
    activeChannels: number | null;
    totalChannels: number | null;
    cost: number | null;
    errors: string[];
}

const EMPTY_EVIDENCE: HomeEvidence = {
    sessions: null,
    jobs: null,
    runningJobs: null,
    automations: null,
    activeChannels: null,
    totalChannels: null,
    cost: null,
    errors: [],
};

function failedLabel(result: PromiseSettledResult<unknown>, label: string): string | null {
    if (result.status === 'fulfilled') return null;
    const error = result.reason;
    const message = error instanceof Error ? error.message : String(error);
    return `${label}: ${message}`;
}

export function ThinClawHome() {
    const { status, source, checkedAt, error, isRefreshing, refresh, capability } = useAgentCockpit();
    const { setActiveThinClawPage } = useChatLayout();
    const [evidence, setEvidence] = useState<HomeEvidence>(EMPTY_EVIDENCE);
    const [isLoadingEvidence, setIsLoadingEvidence] = useState(false);
    const runtimeAvailable = Boolean(status?.engine_running && status?.engine_connected && !error);

    const loadEvidence = useCallback(async () => {
        if (!runtimeAvailable) {
            setEvidence(EMPTY_EVIDENCE);
            return;
        }
        setIsLoadingEvidence(true);
        try {
            const [sessions, jobs, routines, channels, cost] = await Promise.allSettled([
                thinclaw.getThinClawSessions(),
                thinclaw.listJobs(),
                thinclaw.getThinClawCronList(),
                thinclaw.getChannelStatusList(),
                thinclaw.getCostSummary(),
            ]);
            const nextErrors = [
                failedLabel(sessions, 'Sessions'),
                failedLabel(jobs, 'Jobs'),
                failedLabel(routines, 'Automations'),
                failedLabel(channels, 'Channels'),
                failedLabel(cost, 'Usage'),
            ].filter((value): value is string => Boolean(value));
            const jobList = jobs.status === 'fulfilled' ? jobs.value.jobs ?? [] : null;
            const channelList = channels.status === 'fulfilled' ? channels.value : null;
            setEvidence({
                sessions: sessions.status === 'fulfilled' ? sessions.value.sessions.length : null,
                jobs: jobList?.length ?? null,
                runningJobs: jobList ? jobList.filter((job) => ['running', 'in_progress', 'pending', 'queued'].includes(job.state)).length : null,
                automations: routines.status === 'fulfilled' ? routines.value.length : null,
                activeChannels: channelList ? channelList.filter((channel) => channel.state === 'Running').length : null,
                totalChannels: channelList?.length ?? null,
                cost: cost.status === 'fulfilled' ? cost.value.total_cost_usd : null,
                errors: nextErrors,
            });
        } finally {
            setIsLoadingEvidence(false);
        }
    }, [runtimeAvailable]);

    useEffect(() => {
        void loadEvidence();
    }, [loadEvidence]);

    const runtime = capability('runtime');
    const connection = useMemo(() => {
        if (runtime.state === 'loading') return 'Checking';
        if (runtime.state === 'stale') return 'Stale';
        if (status?.engine_running && status?.engine_connected) return 'Connected';
        if (source === 'remote') return 'Disconnected';
        return 'Stopped';
    }, [runtime.state, source, status?.engine_connected, status?.engine_running]);
    const activeProfile = source === 'remote'
        ? status?.profiles.find((profile) => profile.url === status.remote_url)?.name ?? 'Remote profile'
        : 'Local Core';
    const checkedLabel = checkedAt
        ? `Status checked ${new Date(checkedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`
        : 'Status has not returned yet';

    return (
        <AgentPageShell
            eyebrow="Agent Cockpit"
            title="Home"
            description="The selected profile’s connection, active work, and the next safe remediation."
            badge={<StatusBadge status={connection} />}
            actions={(
                <Button variant="secondary" size="sm" onClick={() => { void refresh(); void loadEvidence(); }} disabled={isRefreshing || isLoadingEvidence}>
                    <RefreshCw className={`size-3.5 ${(isRefreshing || isLoadingEvidence) ? 'animate-spin' : ''}`} aria-hidden="true" />
                    Refresh
                </Button>
            )}
        >
            <div className="mx-auto w-full max-w-7xl space-y-5">
                {runtime.state !== 'available' && (
                    <Notice tone={runtime.state === 'stale' ? 'warning' : 'info'} title={runtime.state === 'stale' ? 'Status needs a refresh' : 'Agent runtime is not ready'}>
                        {runtime.reason} {runtime.remediation}
                    </Notice>
                )}
                {error && runtime.state === 'stale' && (
                    <Notice tone="warning" title="Showing last known profile state">{error}</Notice>
                )}

                <Surface className="p-5">
                    <div className="flex flex-wrap items-start justify-between gap-4">
                        <div>
                            <p className="text-sm font-semibold">{activeProfile}</p>
                            <p className="mt-1 text-xs text-content-muted">
                                {source === 'remote' ? 'Remote profile' : 'Local runtime'} · {checkedLabel}
                            </p>
                        </div>
                        <div className="flex items-center gap-2">
                            <StatusBadge status={connection} />
                            <Button variant="ghost" size="sm" onClick={() => setActiveThinClawPage('operations')}>
                                Operations <ArrowRight className="size-3.5" aria-hidden="true" />
                            </Button>
                        </div>
                    </div>
                </Surface>

                <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
                    <MetricCard
                        label="Live sessions"
                        value={evidence.sessions ?? '—'}
                        detail={evidence.sessions === null ? 'Unavailable until the runtime responds' : 'Persisted agent sessions'}
                        action={<Activity className="size-4 text-content-muted" aria-hidden="true" />}
                    />
                    <MetricCard
                        label="Running jobs"
                        value={evidence.runningJobs ?? '—'}
                        detail={evidence.jobs === null ? 'Job state unavailable' : `${evidence.jobs} total jobs`}
                        action={<Clock3 className="size-4 text-content-muted" aria-hidden="true" />}
                    />
                    <MetricCard
                        label="Channel health"
                        value={evidence.activeChannels === null ? '—' : `${evidence.activeChannels}/${evidence.totalChannels}`}
                        detail={evidence.activeChannels === null ? 'No live channel response' : 'Channels reporting running'}
                        action={<Radio className="size-4 text-content-muted" aria-hidden="true" />}
                    />
                    <MetricCard
                        label="Recorded usage"
                        value={evidence.cost === null ? '—' : `$${evidence.cost.toFixed(2)}`}
                        detail={evidence.cost === null ? 'Usage source unavailable' : 'Recorded agent cost; not provider billing'}
                        action={<CircleDollarSign className="size-4 text-content-muted" aria-hidden="true" />}
                    />
                </div>

                <div className="grid gap-4 lg:grid-cols-2">
                    <Surface className="p-5">
                        <div className="flex items-center gap-2">
                            <Timer className="size-4 text-primary" aria-hidden="true" />
                            <h2 className="text-sm font-semibold">Automation readiness</h2>
                        </div>
                        <p className="mt-2 text-sm text-content-muted">
                            {evidence.automations === null
                                ? 'Automation data is unavailable for this profile.'
                                : `${evidence.automations} configured automation${evidence.automations === 1 ? '' : 's'} are visible to this profile.`}
                        </p>
                        <Button className="mt-4" size="sm" variant="secondary" onClick={() => setActiveThinClawPage('automations')}>Open automations</Button>
                    </Surface>
                    <Surface className="p-5">
                        <div className="flex items-center gap-2">
                            <ShieldAlert className="size-4 text-amber-600 dark:text-amber-300" aria-hidden="true" />
                            <h2 className="text-sm font-semibold">Data quality</h2>
                        </div>
                        {evidence.errors.length === 0 ? (
                            <p className="mt-2 text-sm text-content-muted">Each available summary above came from a live command response.</p>
                        ) : (
                            <ul className="mt-2 space-y-1 text-xs text-content-muted">
                                {evidence.errors.map((item) => <li key={item}>{item}</li>)}
                            </ul>
                        )}
                    </Surface>
                </div>
            </div>
        </AgentPageShell>
    );
}
