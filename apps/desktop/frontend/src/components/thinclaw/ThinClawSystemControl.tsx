import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
    AlertTriangle,
    Copy,
    Pause,
    Play,
    RefreshCw,
    Square,
    Terminal,
    Trash2,
} from 'lucide-react';
import { toast } from 'sonner';

import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';
import { useThinClawEvents } from '../../hooks/use-thinclaw-stream';
import { AsyncState, Button, ConfirmDialog, Notice, Surface, Tabs } from '../ui';
import { useOptionalAgentCockpit } from './AgentCockpitProvider';

interface LogLine {
    timestamp: string;
    level: string;
    target: string;
    message: string;
}

type SystemTab = 'gateway' | 'logs';

const LEVEL_STYLES: Record<string, string> = {
    TRACE: 'text-muted-foreground',
    DEBUG: 'text-primary',
    INFO: 'text-emerald-500',
    WARN: 'text-amber-500',
    ERROR: 'text-destructive',
};

function formatLogTime(timestamp: string) {
    const date = new Date(timestamp);
    return Number.isNaN(date.getTime()) ? timestamp : date.toLocaleTimeString();
}

export function ThinClawSystemControl() {
    const [activeTab, setActiveTab] = useState<SystemTab>('gateway');
    const cockpit = useOptionalAgentCockpit();
    const [fallbackStatus, setFallbackStatus] = useState<thinclaw.ThinClawStatus | null>(null);
    const [logs, setLogs] = useState<LogLine[]>([]);
    const [filter, setFilter] = useState('');
    const [isLoading, setIsLoading] = useState(true);
    const [isRefreshingLogs, setIsRefreshingLogs] = useState(false);
    const [isMutatingGateway, setIsMutatingGateway] = useState(false);
    const [stopDialogOpen, setStopDialogOpen] = useState(false);
    const [autoScroll, setAutoScroll] = useState(true);
    const logEndRef = useRef<HTMLDivElement>(null);
    const logContainerRef = useRef<HTMLDivElement>(null);

    const status = cockpit?.status ?? fallbackStatus;

    const loadStatus = useCallback(async (): Promise<thinclaw.ThinClawStatus | null> => {
        if (cockpit) {
            const next = await cockpit.refresh();
            setIsLoading(false);
            return next;
        }
        try {
            const next = await thinclaw.getThinClawStatus();
            setFallbackStatus(next);
            return next;
        } catch (error) {
            toast.error(`Unable to read gateway status: ${String(error)}`);
            return null;
        } finally {
            setIsLoading(false);
        }
    }, [cockpit]);

    const loadLogs = useCallback(async () => {
        setIsRefreshingLogs(true);
        try {
            const data = await thinclaw.getThinClawLogsTail(500);
            setLogs(Array.isArray((data as { logs?: LogLine[] }).logs) ? (data as { logs: LogLine[] }).logs : []);
        } catch (error) {
            toast.error(`Unable to load gateway logs: ${String(error)}`);
        } finally {
            setIsRefreshingLogs(false);
        }
    }, []);

    useEffect(() => {
        if (!cockpit) void loadStatus();
        else setIsLoading(false);
    }, [cockpit, loadStatus]);

    useEffect(() => {
        if (activeTab === 'logs') loadLogs();
    }, [activeTab, loadLogs]);

    useThinClawEvents((payload) => {
        if (payload.kind !== 'LogEntry') return;
        setLogs((current) => {
            const next = [...current, {
                timestamp: payload.timestamp,
                level: payload.level,
                target: payload.target,
                message: payload.message,
            }];
            return next.length > 2000 ? next.slice(-2000) : next;
        });
    }, activeTab === 'logs');

    useEffect(() => {
        if (activeTab === 'logs' && autoScroll) {
            logEndRef.current?.scrollIntoView({ block: 'end' });
        }
    }, [activeTab, autoScroll, logs]);

    const applyGatewayAction = async (wasRunning: boolean) => {
        setIsMutatingGateway(true);
        try {
            if (wasRunning) {
                await thinclaw.stopThinClawGateway();
            } else {
                await thinclaw.startThinClawGateway();
            }
            setStopDialogOpen(false);
            const next = await loadStatus();
            if (next?.engine_running === !wasRunning) {
                toast.success(wasRunning ? 'Gateway stopped' : 'Gateway started');
            } else {
                toast.info(wasRunning
                    ? 'Stop request completed, but the gateway still reports as running.'
                    : 'Start request completed, but the gateway is not running yet.');
            }
        } catch (error) {
            toast.error(`Gateway action failed: ${String(error)}`);
        } finally {
            setIsMutatingGateway(false);
        }
    };

    const toggleGateway = () => {
        if (status?.engine_running) {
            setStopDialogOpen(true);
            return;
        }
        void applyGatewayAction(false);
    };

    const filteredLogs = useMemo(() => {
        const query = filter.trim().toLowerCase();
        if (!query) return logs;
        return logs.filter((entry) => (
            entry.message.toLowerCase().includes(query)
            || entry.target.toLowerCase().includes(query)
            || entry.level.toLowerCase().includes(query)
        ));
    }, [filter, logs]);

    const copyLogs = async () => {
        try {
            await navigator.clipboard.writeText(logs.map((entry) => (
                `${entry.timestamp} [${entry.level}] ${entry.target} ${entry.message}`
            )).join('\n'));
            toast.success('Visible gateway log buffer copied');
        } catch {
            toast.error('Unable to copy gateway logs');
        }
    };

    if (isLoading || Boolean(cockpit && !cockpit.status && !cockpit.error)) {
        return <AsyncState kind="loading" title="Loading gateway operations" className="flex-1" />;
    }

    return (
        <section className="flex min-h-0 flex-1 flex-col overflow-hidden bg-surface-canvas p-6 text-content-primary">
            <header className="mb-6 flex flex-wrap items-start justify-between gap-4">
                <div>
                    <p className="text-[10px] font-bold uppercase tracking-widest text-content-muted">Operations &amp; Safety</p>
                    <h2 className="mt-1 text-xl font-semibold">Gateway and logs</h2>
                    <p className="mt-1 text-sm text-content-muted">
                        Start or stop the embedded gateway and inspect its bounded live log buffer.
                    </p>
                </div>
                <Tabs
                    ariaLabel="Gateway operations"
                    value={activeTab}
                    onValueChange={setActiveTab}
                    tabs={[{ id: 'gateway', label: 'Gateway' }, { id: 'logs', label: 'Logs' }]}
                />
            </header>

            {cockpit?.error && (
                <Notice tone="warning" title="Gateway status is unavailable" className="mb-4">
                    {cockpit.error} The last known state is not shown as a current gateway state.
                </Notice>
            )}

            {activeTab === 'gateway' ? (
                <div className="grid max-w-4xl gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
                    <Surface className="p-5">
                        <div className="flex items-start justify-between gap-4">
                            <div>
                                <p className="text-sm font-semibold">Gateway lifecycle</p>
                                <p className="mt-1 text-xs leading-relaxed text-content-muted">
                                    {status?.engine_running
                                        ? 'The embedded gateway is running. Stopping it ends active agent work and disconnects local control surfaces.'
                                        : 'The embedded gateway is stopped. Start it to use local agent capabilities.'}
                                </p>
                            </div>
                            <span className={cn(
                                'inline-flex shrink-0 items-center rounded-full border px-2.5 py-1 text-[10px] font-semibold',
                                status?.engine_running
                                    ? 'border-emerald-500/30 bg-emerald-500/10 text-emerald-500'
                                    : 'border-surface-outline bg-surface-subtle text-content-muted',
                            )}>
                                {status?.engine_running ? 'Running' : 'Stopped'}
                            </span>
                        </div>
                        <Button
                            className="mt-5 w-full"
                            variant={status?.engine_running ? 'danger' : 'primary'}
                            onClick={toggleGateway}
                            disabled={isMutatingGateway}
                        >
                            {isMutatingGateway
                                ? <RefreshCw className="size-4 animate-spin" aria-hidden="true" />
                                : status?.engine_running ? <Square className="size-4" aria-hidden="true" /> : <Play className="size-4" aria-hidden="true" />}
                            {isMutatingGateway
                                ? 'Updating gateway…'
                                : status?.engine_running
                                    ? 'Stop gateway'
                                    : 'Start gateway'}
                        </Button>
                    </Surface>

                    <Surface elevation="subtle" className="p-5">
                        <div className="flex gap-3">
                            <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber-500" aria-hidden="true" />
                            <div>
                                <p className="text-sm font-semibold">Updates are managed by Desktop</p>
                                <p className="mt-1 text-xs leading-relaxed text-content-muted">
                                    The embedded ThinClaw runtime does not support an in-page binary update or rebuild.
                                    Use the Desktop application update flow in Settings when an update is available.
                                </p>
                            </div>
                        </div>
                    </Surface>
                </div>
            ) : (
                <Surface elevation="elevated" className="flex min-h-0 flex-1 flex-col overflow-hidden">
                    <div className="flex flex-wrap items-center gap-2 border-b border-surface-outline p-3">
                        <Terminal className="size-4 text-content-muted" aria-hidden="true" />
                        <p className="mr-auto text-sm font-semibold">Gateway logs</p>
                        <input
                            aria-label="Filter gateway logs"
                            value={filter}
                            onChange={(event) => setFilter(event.target.value)}
                            placeholder="Filter logs"
                            className="h-[var(--control-height-compact)] min-w-48 rounded-[var(--radius-control)] border border-surface-outline bg-surface-panel px-3 text-xs outline-hidden focus-visible:ring-2 focus-visible:ring-primary/20"
                        />
                        <Button size="icon" variant="ghost" onClick={() => setAutoScroll((value) => !value)} aria-label={autoScroll ? 'Pause log auto-scroll' : 'Resume log auto-scroll'} title={autoScroll ? 'Pause log auto-scroll' : 'Resume log auto-scroll'}>
                            <Pause className="size-4" aria-hidden="true" />
                        </Button>
                        <Button size="icon" variant="ghost" onClick={copyLogs} aria-label="Copy visible gateway log buffer" title="Copy visible gateway log buffer">
                            <Copy className="size-4" aria-hidden="true" />
                        </Button>
                        <Button size="icon" variant="ghost" onClick={loadLogs} aria-label="Refresh gateway logs" title="Refresh gateway logs" disabled={isRefreshingLogs}>
                            <RefreshCw className={cn('size-4', isRefreshingLogs && 'animate-spin')} aria-hidden="true" />
                        </Button>
                        <Button size="icon" variant="ghost" onClick={() => setLogs([])} aria-label="Clear local gateway log buffer" title="Clear local gateway log buffer">
                            <Trash2 className="size-4" aria-hidden="true" />
                        </Button>
                    </div>
                    <div
                        ref={logContainerRef}
                        onScroll={() => {
                            const element = logContainerRef.current;
                            if (!element) return;
                            setAutoScroll(element.scrollHeight - element.scrollTop - element.clientHeight < 40);
                        }}
                        className="min-h-0 flex-1 overflow-auto p-2"
                    >
                        {filteredLogs.length === 0 ? (
                            <AsyncState
                                kind="empty"
                                compact
                                title={logs.length === 0 ? 'No gateway logs in this buffer' : 'No log entries match this filter'}
                                description={logs.length === 0 ? 'Start the gateway or refresh this tab to load recent entries.' : undefined}
                            />
                        ) : filteredLogs.map((entry, index) => (
                            <div key={`${entry.timestamp}-${index}`} className="grid grid-cols-[auto_auto_minmax(0,1fr)] gap-x-3 rounded px-2 py-1 font-mono text-[11px] hover:bg-surface-subtle">
                                <span className="tabular-nums text-content-muted">{formatLogTime(entry.timestamp)}</span>
                                <span className={cn('font-semibold', LEVEL_STYLES[entry.level] ?? 'text-content-muted')}>{entry.level}</span>
                                <span className="min-w-0 break-words text-content-primary">
                                    <span className="text-content-muted">{entry.target}: </span>{entry.message}
                                </span>
                            </div>
                        ))}
                        <div ref={logEndRef} />
                    </div>
                </Surface>
            )}
            <ConfirmDialog
                open={stopDialogOpen}
                onOpenChange={setStopDialogOpen}
                title="Stop the local gateway?"
                description="Stopping the gateway ends active local agent work and disconnects local control surfaces."
                confirmLabel="Stop gateway"
                onConfirm={() => applyGatewayAction(true)}
                isConfirming={isMutatingGateway}
            />
        </section>
    );
}
