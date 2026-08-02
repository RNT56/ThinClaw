import { useCallback, useEffect, useState } from 'react';
import { MessageSquare, Radio, RefreshCw, Settings2, Signal, WifiOff } from 'lucide-react';
import { toast } from 'sonner';

import * as thinclaw from '../../lib/thinclaw';
import { AsyncState, Button, Notice, StatusBadge, Surface } from '../ui';
import { useAgentCockpit } from './AgentCockpitProvider';

const STREAM_MODES = [
    { value: '', label: 'Full reply' },
    { value: 'edit', label: 'Live edit' },
    { value: 'status', label: 'Typing status' },
    { value: 'chunks', label: 'Chunks' },
] as const;

function channelDescription(channel: thinclaw.ChannelInfo) {
    if (channel.type === 'native') return 'Native adapter managed by the selected runtime.';
    if (channel.type === 'wasm') return 'Adapter supplied through the selected runtime.';
    return 'Built-in adapter supplied by the selected runtime.';
}

/** Live channel inventory only — no synthetic adapters on an empty or failed response. */
export function ThinClawChannels() {
    const { status, source } = useAgentCockpit();
    const [channels, setChannels] = useState<thinclaw.ChannelInfo[]>([]);
    const [error, setError] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(true);
    const [saving, setSaving] = useState<string | null>(null);

    const load = useCallback(async () => {
        setIsLoading(true);
        setError(null);
        try {
            const response = await thinclaw.getThinClawChannelsList();
            setChannels(Array.isArray(response.channels) ? response.channels : []);
        } catch (caught) {
            setChannels([]);
            setError(caught instanceof Error ? caught.message : String(caught));
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        void load();
    }, [load]);

    const updateStreamMode = async (channel: thinclaw.ChannelInfo, streamMode: string) => {
        setSaving(channel.id);
        try {
            const result = await thinclaw.setSetting(`channels.${channel.id}_stream_mode`, streamMode);
            if (!result.ok) {
                toast.error('The channel setting was not accepted by the selected profile.');
                return;
            }
            setChannels((current) => current.map((item) => item.id === channel.id ? { ...item, stream_mode: streamMode } : item));
            toast.success('Stream behavior saved');
        } catch (caught) {
            toast.error(caught instanceof Error ? caught.message : String(caught));
        } finally {
            setSaving(null);
        }
    };

    if (isLoading) return <AsyncState kind="loading" title="Loading channel inventory" className="flex-1" />;
    if (error) return <AsyncState kind="error" title="Channel inventory is unavailable" description={error} actionLabel="Retry" onAction={() => void load()} className="flex-1" />;

    const running = channels.filter((channel) => channel.enabled).length;
    return (
        <div className="mx-auto flex w-full max-w-7xl flex-1 flex-col gap-5">
            <Notice tone="info" title="Live inventory only">
                {source === 'remote' ? 'This list comes from the selected remote profile.' : 'This list comes from the local runtime.'} Empty means the runtime returned no configured or discoverable channels; Desktop does not add example adapters.
            </Notice>
            <div className="flex flex-wrap items-center justify-between gap-3">
                <p className="text-sm text-content-muted">{running} running of {channels.length} reported channels</p>
                <Button size="sm" variant="secondary" onClick={() => void load()}>
                    <RefreshCw className="size-3.5" aria-hidden="true" /> Refresh
                </Button>
            </div>
            {channels.length === 0 ? (
                <AsyncState kind="empty" title="No channels reported" description="Open Setup to configure an adapter that exposes a supported schema." />
            ) : (
                <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-3">
                    {channels.map((channel) => {
                        const streamSupported = ['discord', 'telegram'].includes(channel.id);
                        return (
                            <Surface key={channel.id} className="flex min-w-0 flex-col p-5">
                                <div className="flex items-start justify-between gap-3">
                                    <div className="flex min-w-0 items-center gap-3">
                                        <div className="grid size-9 shrink-0 place-items-center rounded-[var(--radius-control)] bg-primary/10 text-primary">
                                            <MessageSquare className="size-4" aria-hidden="true" />
                                        </div>
                                        <div className="min-w-0">
                                            <h2 className="truncate text-sm font-semibold">{channel.name}</h2>
                                            <p className="mt-0.5 text-[10px] uppercase tracking-widest text-content-muted">{channel.type}</p>
                                        </div>
                                    </div>
                                    <StatusBadge status={channel.enabled ? 'Running' : 'Stopped'} />
                                </div>
                                <p className="mt-4 text-xs leading-relaxed text-content-muted">{channelDescription(channel)}</p>
                                {streamSupported ? (
                                    <label className="mt-4 block text-xs font-medium text-content-primary">
                                        Stream behavior
                                        <select
                                            aria-label={`${channel.name} stream behavior`}
                                            value={channel.stream_mode ?? ''}
                                            disabled={saving === channel.id}
                                            onChange={(event) => void updateStreamMode(channel, event.currentTarget.value)}
                                            className="mt-1.5 h-[var(--control-height-compact)] w-full rounded-[var(--radius-control)] border border-surface-outline bg-surface-subtle px-3 text-xs text-content-primary outline-none focus-visible:ring-2 focus-visible:ring-primary/20"
                                        >
                                            {STREAM_MODES.map((mode) => <option key={mode.value} value={mode.value}>{mode.label}</option>)}
                                        </select>
                                    </label>
                                ) : (
                                    <div className="mt-4 flex items-center gap-2 text-xs text-content-muted">
                                        <Signal className="size-3.5" aria-hidden="true" /> Stream behavior is not exposed by this adapter.
                                    </div>
                                )}
                            </Surface>
                        );
                    })}
                </div>
            )}
            {!status?.engine_running && (
                <div className="flex items-center gap-2 text-xs text-content-muted"><WifiOff className="size-3.5" aria-hidden="true" /> The last inventory may be unavailable while the gateway is stopped.</div>
            )}
            <div className="flex items-center gap-2 text-xs text-content-muted"><Radio className="size-3.5" aria-hidden="true" /> Use Setup for fields that the runtime explicitly publishes. Secret values are never displayed here.</div>
            <div className="flex items-center gap-2 text-xs text-content-muted"><Settings2 className="size-3.5" aria-hidden="true" /> Pairing and adapter security are kept in their dedicated Channel Center tab.</div>
        </div>
    );
}
