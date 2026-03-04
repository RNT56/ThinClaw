import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    RefreshCw, MessageSquare, Send, Hash, Shield, Globe,
    Radio, Smartphone, Wifi, ChevronDown, ChevronUp,
    Settings2, Podcast, Zap, AlertCircle, ArrowUpDown
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';
import { listen } from '@tauri-apps/api/event';

// ── Channel icon mapping ────────────────────────────────────────
const CHANNEL_ICONS: Record<string, any> = {
    slack: MessageSquare,
    telegram: Send,
    discord: Hash,
    signal: Shield,
    webhook: Globe,
    nostr: Radio,
    whatsapp: Smartphone,
};

const STATE_COLORS: Record<string, { bg: string; text: string; dot: string }> = {
    Running: { bg: 'bg-emerald-500/10', text: 'text-emerald-400', dot: 'bg-emerald-500' },
    Connecting: { bg: 'bg-amber-500/10', text: 'text-amber-400', dot: 'bg-amber-500 animate-pulse' },
    Degraded: { bg: 'bg-orange-500/10', text: 'text-orange-400', dot: 'bg-orange-500' },
    Disconnected: { bg: 'bg-zinc-500/10', text: 'text-zinc-400', dot: 'bg-zinc-500' },
    Error: { bg: 'bg-red-500/10', text: 'text-red-400', dot: 'bg-red-500' },
};

const STREAM_MODES = ['', 'full', 'typing_only', 'disabled'];
const STREAM_MODE_LABELS: Record<string, string> = {
    '': 'Default',
    'full': 'Full Streaming',
    'typing_only': 'Typing Only',
    'disabled': 'Disabled',
};

// ── Status Card ────────────────────────────────────────────────
function ChannelStatusCard({
    entry,
    expanded,
    onToggle,
    onStreamChange,
}: {
    entry: openclaw.ChannelStatusEntry;
    expanded: boolean;
    onToggle: () => void;
    onStreamChange: (id: string, mode: string) => void;
}) {
    const Icon = CHANNEL_ICONS[entry.id] || Wifi;
    const stateStyle = STATE_COLORS[entry.state] || STATE_COLORS.Disconnected;
    const hasStreamMode = ['discord', 'telegram', 'slack'].includes(entry.id);

    return (
        <motion.div
            layout
            className={cn(
                "rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm transition-all",
                entry.enabled ? "border-primary/20 shadow-primary/5" : "border-white/10"
            )}
        >
            <div className="p-5">
                {/* Header row */}
                <div className="flex items-start justify-between mb-3">
                    <div className="flex items-center gap-3">
                        <div className={cn(
                            "p-2.5 rounded-xl border",
                            entry.enabled ? "bg-primary/10 border-primary/20" : "bg-white/5 border-white/10"
                        )}>
                            <Icon className={cn("w-5 h-5", entry.enabled ? "text-primary" : "text-muted-foreground")} />
                        </div>
                        <div>
                            <h3 className="font-semibold">{entry.name}</h3>
                            <span className={cn(
                                "text-[10px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded",
                                entry.type === 'native' ? "bg-purple-500/10 text-purple-400" :
                                    entry.type === 'wasm' ? "bg-blue-500/10 text-blue-400" :
                                        "bg-green-500/10 text-green-400"
                            )}>
                                {entry.type}
                            </span>
                        </div>
                    </div>

                    {/* State badge */}
                    <div className={cn("flex items-center gap-1.5 px-2.5 py-1 rounded-full text-[10px] font-bold uppercase tracking-wider border", stateStyle.bg, stateStyle.text, "border-current/20")}>
                        <span className={cn("w-1.5 h-1.5 rounded-full", stateStyle.dot)} />
                        {entry.state}
                    </div>
                </div>

                {/* Metrics row */}
                <div className="grid grid-cols-3 gap-3 mt-4">
                    <div className="text-center p-2 rounded-lg bg-white/[0.02] border border-white/5">
                        <p className="text-lg font-bold tabular-nums">{entry.messages_sent}</p>
                        <p className="text-[9px] text-muted-foreground uppercase tracking-widest">Sent</p>
                    </div>
                    <div className="text-center p-2 rounded-lg bg-white/[0.02] border border-white/5">
                        <p className="text-lg font-bold tabular-nums">{entry.messages_received}</p>
                        <p className="text-[9px] text-muted-foreground uppercase tracking-widest">Received</p>
                    </div>
                    <div className="text-center p-2 rounded-lg bg-white/[0.02] border border-white/5">
                        <p className="text-lg font-bold tabular-nums">
                            {entry.uptime_secs != null ? formatUptime(entry.uptime_secs) : '—'}
                        </p>
                        <p className="text-[9px] text-muted-foreground uppercase tracking-widest">Uptime</p>
                    </div>
                </div>

                {/* Error display */}
                {entry.last_error && (
                    <div className="mt-3 flex items-start gap-2 p-2.5 rounded-lg bg-red-500/5 border border-red-500/10">
                        <AlertCircle className="w-3.5 h-3.5 text-red-400 mt-0.5 shrink-0" />
                        <p className="text-xs text-red-300/80 leading-relaxed">{entry.last_error}</p>
                    </div>
                )}

                {/* Stream mode badge */}
                {hasStreamMode && entry.stream_mode && (
                    <div className="mt-3 flex items-center gap-2">
                        <Podcast className="w-3.5 h-3.5 text-amber-400" />
                        <span className="text-xs text-amber-400/80 font-medium">
                            Stream: {STREAM_MODE_LABELS[entry.stream_mode] || entry.stream_mode}
                        </span>
                    </div>
                )}

                {/* Expand button */}
                {hasStreamMode && entry.enabled && (
                    <button
                        onClick={onToggle}
                        className="mt-4 w-full py-2 rounded-xl text-xs font-medium text-muted-foreground hover:text-foreground hover:bg-white/5 transition-all flex items-center justify-center gap-1.5 border border-white/5"
                    >
                        <Settings2 className="w-3.5 h-3.5" />
                        Stream Settings
                        {expanded ? <ChevronUp className="w-3 h-3" /> : <ChevronDown className="w-3 h-3" />}
                    </button>
                )}
            </div>

            {/* Expanded stream config */}
            <AnimatePresence>
                {expanded && hasStreamMode && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="px-5 pb-5 pt-2 border-t border-white/5 space-y-3">
                            <p className="text-[10px] uppercase font-bold text-muted-foreground tracking-widest">Streaming Mode</p>
                            <div className="grid grid-cols-2 gap-2">
                                {STREAM_MODES.map(mode => (
                                    <button
                                        key={mode}
                                        onClick={() => onStreamChange(entry.id, mode)}
                                        className={cn(
                                            "px-3 py-2 rounded-lg text-xs font-medium transition-all border",
                                            (entry.stream_mode || '') === mode
                                                ? "bg-primary/15 text-primary border-primary/30"
                                                : "bg-white/[0.03] text-muted-foreground hover:bg-white/5 border-white/5"
                                        )}
                                    >
                                        {STREAM_MODE_LABELS[mode]}
                                    </button>
                                ))}
                            </div>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}

function formatUptime(secs: number): string {
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m`;
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return `${h}h ${m}m`;
}

// ── Main Component ──────────────────────────────────────────────
export function OpenClawChannelStatus() {
    const [entries, setEntries] = useState<openclaw.ChannelStatusEntry[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [expandedChannel, setExpandedChannel] = useState<string | null>(null);
    const [sortBy, setSortBy] = useState<'name' | 'state'>('name');

    const fetchData = useCallback(async () => {
        try {
            const data = await openclaw.getChannelStatusList();
            setEntries(data);
        } catch (e) {
            console.error('Failed to fetch channel statuses:', e);
            // Fallback to old API
            try {
                const resp = await openclaw.getOpenClawChannelsList();
                setEntries((resp.channels || []).map(ch => ({
                    ...ch,
                    state: ch.enabled ? 'Running' : 'Disconnected' as any,
                    type: ch.type as any,
                    uptime_secs: null,
                    messages_sent: 0,
                    messages_received: 0,
                    last_error: null,
                })));
            } catch (_) {
                setEntries([]);
            }
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        fetchData();
        const interval = setInterval(fetchData, 15000); // Poll every 15s

        // Also listen for real-time channel events
        const unlisten = listen('openclaw-event', (event: any) => {
            const payload = event.payload;
            if (payload.kind === 'ChannelStatus') {
                setEntries(prev => prev.map(ch =>
                    ch.id === payload.channel_id
                        ? { ...ch, state: payload.state, last_error: payload.error }
                        : ch
                ));
            }
        });

        return () => {
            clearInterval(interval);
            unlisten.then(fn => fn());
        };
    }, [fetchData]);

    const handleStreamModeChange = async (channelId: string, mode: string) => {
        const envKey = `${channelId.toUpperCase()}_STREAM_MODE`;
        try {
            await openclaw.setSetting(envKey, mode);
            setEntries(prev => prev.map(ch =>
                ch.id === channelId ? { ...ch, stream_mode: mode } : ch
            ));
            toast.success(`${channelId} stream mode set to ${STREAM_MODE_LABELS[mode] || mode}`);
        } catch (e) {
            toast.error(`Failed to update stream mode: ${e}`);
        }
    };

    const sorted = [...entries].sort((a, b) => {
        if (sortBy === 'state') {
            const stateOrder = { Running: 0, Connecting: 1, Degraded: 2, Error: 3, Disconnected: 4 };
            return (stateOrder[a.state as keyof typeof stateOrder] ?? 5) - (stateOrder[b.state as keyof typeof stateOrder] ?? 5);
        }
        return a.name.localeCompare(b.name);
    });

    const activeCount = entries.filter(e => e.state === 'Running').length;
    const totalMsgs = entries.reduce((acc, e) => acc + e.messages_sent + e.messages_received, 0);

    if (isLoading) {
        return (
            <div className="flex-1 flex items-center justify-center">
                <RefreshCw className="w-5 h-5 animate-spin text-muted-foreground" />
            </div>
        );
    }

    return (
        <motion.div
            className="flex-1 overflow-y-auto p-8 space-y-6"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
        >
            {/* Header */}
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-3">
                    <div className="p-2.5 rounded-xl bg-indigo-500/10 border border-indigo-500/20">
                        <Zap className="w-5 h-5 text-indigo-400" />
                    </div>
                    <div>
                        <h1 className="text-xl font-bold">Channel Status</h1>
                        <p className="text-xs text-muted-foreground">
                            {activeCount}/{entries.length} channels active · {totalMsgs} total messages
                        </p>
                    </div>
                </div>
                <div className="flex items-center gap-2">
                    <button
                        onClick={() => setSortBy(s => s === 'name' ? 'state' : 'name')}
                        className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                    >
                        <ArrowUpDown className="w-3.5 h-3.5" />
                        Sort: {sortBy === 'name' ? 'Name' : 'Status'}
                    </button>
                    <button
                        onClick={fetchData}
                        className="p-2 rounded-lg text-muted-foreground hover:text-foreground bg-white/[0.03] hover:bg-white/5 border border-white/5 transition-all"
                    >
                        <RefreshCw className={cn("w-3.5 h-3.5", isLoading && "animate-spin")} />
                    </button>
                </div>
            </div>

            {/* Grid */}
            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
                {sorted.map(entry => (
                    <ChannelStatusCard
                        key={entry.id}
                        entry={entry}
                        expanded={expandedChannel === entry.id}
                        onToggle={() => setExpandedChannel(prev => prev === entry.id ? null : entry.id)}
                        onStreamChange={handleStreamModeChange}
                    />
                ))}
            </div>
        </motion.div>
    );
}
