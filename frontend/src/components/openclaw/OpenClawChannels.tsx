import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    MessageSquare,
    Smartphone,
    RefreshCw,
    Shield,
    Send,
    Hash,
    Globe,
    Radio,
    Zap,
    ChevronDown,
    ChevronUp,
    Settings2,
    Podcast,
    Wifi
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

const CHANNEL_DESCRIPTIONS: Record<string, string> = {
    slack: 'Enterprise workspace bridge via Socket Mode. Supports streaming draft replies.',
    telegram: 'Full Telegram Bot API integration with forum topics, channel posts, and DM pairing.',
    discord: 'Native Rust gateway with WebSocket + REST. Streaming draft replies support.',
    signal: 'Encrypted messaging via signal-cli daemon with SSE listener.',
    webhook: 'HTTP webhook endpoint with HMAC-SHA256 signature verification. Always active.',
    nostr: 'NIP-04 encrypted direct messages on the Nostr protocol.',
    whatsapp: 'Bridge to WhatsApp via web authentication. Supports media and group chats.',
};

const STREAM_MODES = ['', 'full', 'typing_only', 'disabled'];
const STREAM_MODE_LABELS: Record<string, string> = {
    '': 'Default',
    'full': 'Full Streaming',
    'typing_only': 'Typing Only',
    'disabled': 'Disabled',
};

// ── Channel Card  ────────────────────────────────────────────────
interface ChannelCardProps {
    channel: openclaw.ChannelInfo;
    onConfigureStream?: (id: string, mode: string) => void;
    hasStreamMode: boolean;
    expanded: boolean;
    onToggleExpand: () => void;
}

function ChannelCard({ channel, onConfigureStream, hasStreamMode, expanded, onToggleExpand }: ChannelCardProps) {
    const Icon = CHANNEL_ICONS[channel.id] || Wifi;
    const description = CHANNEL_DESCRIPTIONS[channel.id] || `${channel.name} channel (${channel.type})`;

    return (
        <motion.div
            layout
            className={cn(
                "rounded-2xl border bg-card/30 backdrop-blur-md shadow-sm transition-all",
                channel.enabled
                    ? "border-primary/20 shadow-primary/5"
                    : "border-white/10"
            )}
        >
            <div className="p-6">
                <div className="flex items-start justify-between mb-4">
                    <div className="flex items-center gap-3">
                        <div className={cn(
                            "p-2.5 rounded-xl border",
                            channel.enabled
                                ? "bg-primary/10 border-primary/20"
                                : "bg-white/5 border-white/10"
                        )}>
                            <Icon className={cn("w-5 h-5", channel.enabled ? "text-primary" : "text-muted-foreground")} />
                        </div>
                        <div>
                            <h3 className="font-semibold">{channel.name}</h3>
                            <span className={cn(
                                "text-[10px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded",
                                channel.type === 'native' ? "bg-purple-500/10 text-purple-400" :
                                    channel.type === 'wasm' ? "bg-blue-500/10 text-blue-400" :
                                        "bg-green-500/10 text-green-400"
                            )}>
                                {channel.type}
                            </span>
                        </div>
                    </div>
                    <div className="flex items-center gap-2">
                        <div className={cn(
                            "px-2.5 py-1 rounded-full text-[10px] font-bold uppercase tracking-wider border",
                            channel.enabled
                                ? "text-green-500 bg-green-500/10 border-green-500/20"
                                : "text-muted-foreground bg-white/5 border-white/10"
                        )}>
                            {channel.enabled ? 'Active' : 'Inactive'}
                        </div>
                    </div>
                </div>

                <p className="text-sm text-muted-foreground leading-relaxed">{description}</p>

                {/* Stream mode badge */}
                {hasStreamMode && channel.stream_mode && (
                    <div className="mt-3 flex items-center gap-2">
                        <Podcast className="w-3.5 h-3.5 text-amber-400" />
                        <span className="text-xs text-amber-400/80 font-medium">
                            Stream: {STREAM_MODE_LABELS[channel.stream_mode] || channel.stream_mode}
                        </span>
                    </div>
                )}

                {/* Expand button for stream config */}
                {hasStreamMode && channel.enabled && (
                    <button
                        onClick={onToggleExpand}
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
                        <div className="px-6 pb-6 pt-2 border-t border-white/5 space-y-3">
                            <p className="text-[10px] uppercase font-bold text-muted-foreground tracking-widest">Streaming Mode</p>
                            <div className="grid grid-cols-2 gap-2">
                                {STREAM_MODES.map(mode => (
                                    <button
                                        key={mode}
                                        onClick={() => onConfigureStream?.(channel.id, mode)}
                                        className={cn(
                                            "px-3 py-2 rounded-lg text-xs font-medium transition-all border",
                                            (channel.stream_mode || '') === mode
                                                ? "bg-primary/15 text-primary border-primary/30"
                                                : "bg-white/[0.03] text-muted-foreground hover:bg-white/5 border-white/5"
                                        )}
                                    >
                                        {STREAM_MODE_LABELS[mode]}
                                    </button>
                                ))}
                            </div>
                            <p className="text-[10px] text-muted-foreground/60 leading-relaxed">
                                Controls how the agent streams partial replies in this channel.
                                &quot;Full&quot; sends incremental edits, &quot;Typing Only&quot; shows a typing indicator.
                            </p>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}

// ── Main Component ──────────────────────────────────────────────
export function OpenClawChannels() {
    const [channels, setChannels] = useState<openclaw.ChannelInfo[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [expandedChannel, setExpandedChannel] = useState<string | null>(null);
    const [qrCode, setQrCode] = useState<string | null>(null);
    const [_waStatus, setWaStatus] = useState<'connected' | 'disconnected' | 'authenticating' | 'error'>('disconnected');

    const fetchChannels = useCallback(async () => {
        try {
            const resp = await openclaw.getOpenClawChannelsList();
            setChannels(resp.channels || []);
        } catch (e) {
            console.error('Failed to fetch channels:', e);
            // Fallback: use status API
            try {
                const status = await openclaw.getOpenClawStatus();
                setChannels([
                    { id: 'slack', name: 'Slack', type: 'wasm', enabled: status.slack_enabled, stream_mode: '' },
                    { id: 'telegram', name: 'Telegram', type: 'wasm', enabled: status.telegram_enabled, stream_mode: '' },
                    { id: 'discord', name: 'Discord', type: 'native', enabled: false, stream_mode: '' },
                    { id: 'webhook', name: 'HTTP Webhook', type: 'builtin', enabled: true, stream_mode: '' },
                ]);
            } catch (_e) {
                // Final fallback
                setChannels([]);
            }
        } finally {
            setIsLoading(false);
        }
    }, []);

    useEffect(() => {
        fetchChannels();

        // Listen for login events (QR codes, etc)
        const unlisten = listen('openclaw-event', (event: any) => {
            const payload = event.payload;
            if (payload.kind === 'WebLogin') {
                if (payload.provider === 'whatsapp') {
                    if (payload.qr_code) {
                        setQrCode(payload.qr_code);
                        setWaStatus('authenticating');
                    }
                    if (payload.status === 'connected') {
                        setWaStatus('connected');
                        setQrCode(null);
                        toast.success('WhatsApp connected successfully');
                    }
                    if (payload.status === 'error') {
                        setWaStatus('error');
                        toast.error('WhatsApp connection failed');
                    }
                }
            }
        });

        return () => { unlisten.then(fn => fn()); };
    }, [fetchChannels]);

    const handleStreamModeChange = async (channelId: string, mode: string) => {
        const envKey = `${channelId.toUpperCase()}_STREAM_MODE`;
        try {
            await openclaw.setSetting(envKey, mode);
            // Update local state
            setChannels(prev => prev.map(ch =>
                ch.id === channelId ? { ...ch, stream_mode: mode } : ch
            ));
            toast.success(`${channelId} stream mode set to ${STREAM_MODE_LABELS[mode] || mode}`);
        } catch (e) {
            toast.error(`Failed to update stream mode: ${e}`);
        }
    };

    const streamChannels = ['discord', 'telegram', 'slack'];
    const activeChannels = channels.filter(ch => ch.enabled);
    const inactiveChannels = channels.filter(ch => !ch.enabled);

    if (isLoading) {
        return (
            <div className="flex-1 flex items-center justify-center p-8">
                <RefreshCw className="w-8 h-8 text-primary animate-spin" />
            </div>
        );
    }

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 p-8 space-y-8 max-w-6xl mx-auto"
        >
            <div className="flex items-center justify-between">
                <div>
                    <h1 className="text-3xl font-bold tracking-tight">Channel Handshakes</h1>
                    <p className="text-muted-foreground mt-1">
                        All configured messaging channels.
                        <span className="ml-2 text-primary font-medium">{activeChannels.length} active</span>
                        <span className="ml-1 text-muted-foreground/50">/ {channels.length} total</span>
                    </p>
                </div>
                <button
                    onClick={() => { setIsLoading(true); fetchChannels(); }}
                    className="p-2.5 rounded-lg bg-card border border-white/10 hover:bg-white/5 transition-colors"
                >
                    <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                </button>
            </div>

            {/* Active channels */}
            {activeChannels.length > 0 && (
                <div className="space-y-4">
                    <h2 className="text-xs font-bold uppercase tracking-widest text-muted-foreground flex items-center gap-2">
                        <Zap className="w-3.5 h-3.5 text-green-500" />
                        Active Channels
                    </h2>
                    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                        {activeChannels.map(ch => (
                            <ChannelCard
                                key={ch.id}
                                channel={ch}
                                hasStreamMode={streamChannels.includes(ch.id)}
                                expanded={expandedChannel === ch.id}
                                onToggleExpand={() => setExpandedChannel(prev => prev === ch.id ? null : ch.id)}
                                onConfigureStream={handleStreamModeChange}
                            />
                        ))}
                    </div>
                </div>
            )}

            {/* Inactive channels */}
            {inactiveChannels.length > 0 && (
                <div className="space-y-4">
                    <h2 className="text-xs font-bold uppercase tracking-widest text-muted-foreground">
                        Available Channels
                    </h2>
                    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
                        {inactiveChannels.map(ch => (
                            <ChannelCard
                                key={ch.id}
                                channel={ch}
                                hasStreamMode={streamChannels.includes(ch.id)}
                                expanded={expandedChannel === ch.id}
                                onToggleExpand={() => setExpandedChannel(prev => prev === ch.id ? null : ch.id)}
                                onConfigureStream={handleStreamModeChange}
                            />
                        ))}
                    </div>
                </div>
            )}

            {/* QR Code Modal for WhatsApp */}
            <AnimatePresence>
                {qrCode && (
                    <div className="fixed inset-0 bg-black/60 backdrop-blur-sm z-50 flex items-center justify-center p-4">
                        <motion.div
                            initial={{ scale: 0.9, opacity: 0 }}
                            animate={{ scale: 1, opacity: 1 }}
                            exit={{ scale: 0.9, opacity: 0 }}
                            className="bg-card border border-white/10 rounded-3xl p-8 max-w-sm w-full text-center shadow-2xl shadow-black/50"
                        >
                            <h2 className="text-2xl font-bold mb-2">Scan QR Code</h2>
                            <p className="text-sm text-muted-foreground mb-6">Open WhatsApp on your phone and scan this code to link your device.</p>

                            <div className="bg-white p-4 rounded-2xl mx-auto w-fit mb-6 shadow-inner">
                                <img
                                    src={`https://api.qrserver.com/v1/create-qr-code/?size=250x250&data=${encodeURIComponent(qrCode)}`}
                                    alt="WhatsApp QR Code"
                                    className="w-48 h-48 block"
                                />
                            </div>

                            <div className="flex flex-col gap-3">
                                <div className="flex items-center justify-center gap-2 text-blue-400 animate-pulse text-sm font-medium">
                                    <RefreshCw className="w-3.5 h-3.5 animate-spin" />
                                    Waiting for scan...
                                </div>
                                <button
                                    onClick={() => setQrCode(null)}
                                    className="text-xs text-muted-foreground hover:text-foreground transition-colors mt-2"
                                >
                                    Cancel Authentication
                                </button>
                            </div>
                        </motion.div>
                    </div>
                )}
            </AnimatePresence>

            {/* Info Notice */}
            <div className="p-6 rounded-2xl border bg-blue-500/5 border-blue-500/20 flex gap-4">
                <div className="p-2 bg-blue-500/10 rounded-xl h-fit">
                    <Shield className="w-5 h-5 text-blue-500" />
                </div>
                <div>
                    <h4 className="text-sm font-semibold text-blue-500 uppercase tracking-wider">Channel Security</h4>
                    <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                        Communications are proxied through the IronClaw Gateway.
                        Configure channels via environment variables or the config editor.
                        Streaming draft replies are supported on Discord, Telegram, and Slack.
                    </p>
                </div>
            </div>
        </motion.div>
    );
}
