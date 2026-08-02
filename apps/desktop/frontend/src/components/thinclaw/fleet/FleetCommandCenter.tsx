import { useCallback, useEffect, useState } from 'react';
import { Megaphone, RefreshCw, Send, Users } from 'lucide-react';
import { toast } from 'sonner';

import * as thinclaw from '../../../lib/thinclaw';
import { Button, Notice, StatusBadge, Surface } from '../../ui';

function connectionLabel(agent: thinclaw.AgentStatusSummary) {
    if (!agent.online) return 'Offline';
    if (agent.run_status === 'error') return 'Error';
    if (agent.run_status === 'idle') return 'Connected';
    return agent.run_status ?? 'Unknown';
}

/**
 * Fleet intentionally contains only observations and broadcast receipts. The
 * desktop has no target-profile spawn/abort protocol, so it never presents a
 * selected profile as an execution target.
 */
export function FleetCommandCenter() {
    const [agents, setAgents] = useState<thinclaw.AgentStatusSummary[]>([]);
    const [error, setError] = useState<string | null>(null);
    const [checkedAt, setCheckedAt] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [broadcast, setBroadcast] = useState('');
    const [isBroadcasting, setIsBroadcasting] = useState(false);
    const [deliveries, setDeliveries] = useState<thinclaw.FleetBroadcastDelivery[] | null>(null);

    const load = useCallback(async () => {
        setIsLoading(true);
        try {
            const next = await thinclaw.getFleetStatus();
            setAgents(next);
            setError(null);
            setCheckedAt(new Date().toISOString());
        } catch (caught) {
            setAgents([]);
            setError(caught instanceof Error ? caught.message : String(caught));
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        void load();
        const poll = window.setInterval(() => {
            if (document.visibilityState === 'visible') void load();
        }, 15_000);
        return () => window.clearInterval(poll);
    }, [load]);

    const sendBroadcast = async () => {
        const command = broadcast.trim();
        if (!command) return;
        setIsBroadcasting(true);
        setDeliveries(null);
        try {
            const result = await thinclaw.broadcastCommand(command);
            setDeliveries(result.deliveries);
            setBroadcast('');
            if (result.failed === 0) toast.success(`Broadcast receipt confirmed for ${result.delivered} profile${result.delivered === 1 ? '' : 's'}`);
            else toast.warning(`Broadcast reached ${result.delivered} of ${result.attempted} profiles`);
            await load();
        } catch (caught) {
            toast.error(caught instanceof Error ? caught.message : String(caught));
        } finally {
            setIsBroadcasting(false);
        }
    };

    return (
        <div className="mx-auto flex w-full max-w-7xl flex-1 flex-col gap-5">
            <Notice tone="warning" title="Broadcast only">
                Targeted task dispatch and target-profile abort are unavailable in Desktop. A broadcast is sent once to every configured profile and returns a delivery receipt for each target; use it only for instructions that are safe for all recipients.
            </Notice>
            <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex items-center gap-2"><Users className="size-4 text-primary" aria-hidden="true" /><p className="text-sm text-content-muted">{agents.length} reported profile{agents.length === 1 ? '' : 's'}{checkedAt ? ` · checked ${new Date(checkedAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}` : ''}</p></div>
                <Button size="sm" variant="secondary" onClick={() => void load()} disabled={isLoading}><RefreshCw className={`size-3.5 ${isLoading ? 'animate-spin' : ''}`} aria-hidden="true" />Refresh</Button>
            </div>
            {error ? (
                <Notice tone="error" title="Fleet status is unavailable">{error}</Notice>
            ) : agents.length === 0 && !isLoading ? (
                <Surface className="p-5 text-sm text-content-muted">No local runtime or remote agent profile returned fleet status.</Surface>
            ) : (
                <Surface className="overflow-x-auto">
                    <table className="w-full min-w-[44rem] text-left text-xs">
                        <thead className="border-b border-surface-outline text-[10px] font-bold uppercase tracking-widest text-content-muted">
                            <tr><th className="px-4 py-3">Profile</th><th className="px-4 py-3">Connection</th><th className="px-4 py-3">Latency</th><th className="px-4 py-3">Model</th><th className="px-4 py-3">Reported capabilities</th></tr>
                        </thead>
                        <tbody>
                            {agents.map((agent) => (
                                <tr key={agent.id} className="border-b border-surface-outline last:border-0">
                                    <td className="px-4 py-3"><p className="font-semibold text-content-primary">{agent.name}</p><p className="mt-0.5 max-w-56 truncate font-mono text-[10px] text-content-muted">{agent.url}</p></td>
                                    <td className="px-4 py-3"><StatusBadge status={connectionLabel(agent)} /></td>
                                    <td className="px-4 py-3 tabular-nums text-content-muted">{agent.latency_ms === null ? '—' : `${agent.latency_ms} ms`}</td>
                                    <td className="max-w-52 truncate px-4 py-3 font-mono text-[11px] text-content-muted" title={agent.model ?? undefined}>{agent.model ?? '—'}</td>
                                    <td className="px-4 py-3 text-content-muted">{agent.capabilities?.length ? agent.capabilities.join(', ') : '—'}</td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </Surface>
            )}
            <Surface className="p-5">
                <div className="flex items-center gap-2"><Megaphone className="size-4 text-primary" aria-hidden="true" /><h2 className="text-sm font-semibold">Broadcast to all configured profiles</h2></div>
                <textarea aria-label="Broadcast instruction" value={broadcast} onChange={(event) => setBroadcast(event.currentTarget.value)} maxLength={4000} rows={3} placeholder="An instruction safe for every configured agent…" className="mt-3 w-full rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle p-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-primary/20" />
                <div className="mt-3 flex flex-wrap items-center justify-between gap-3"><p className="text-xs text-content-muted">{broadcast.length}/4,000 characters</p><Button variant="primary" size="sm" onClick={() => void sendBroadcast()} disabled={!broadcast.trim() || isBroadcasting}><Send className="size-3.5" aria-hidden="true" />{isBroadcasting ? 'Sending…' : 'Broadcast'}</Button></div>
            </Surface>
            {deliveries && <Surface className="p-5"><h2 className="text-sm font-semibold">Latest delivery receipts</h2><ul className="mt-3 space-y-2 text-xs">{deliveries.map((delivery) => <li key={delivery.agent_id} className="flex flex-wrap items-center justify-between gap-2"><span className="font-medium text-content-primary">{delivery.agent_name}</span><span className={delivery.delivered ? 'text-emerald-700 dark:text-emerald-300' : 'text-destructive'}>{delivery.delivered ? 'Delivered' : delivery.error ?? 'Not delivered'}</span></li>)}</ul></Surface>}
        </div>
    );
}
