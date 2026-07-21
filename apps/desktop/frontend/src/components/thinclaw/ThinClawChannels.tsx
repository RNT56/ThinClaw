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
    Wifi,
    Mail,
    ExternalLink,
    Loader2,
    CheckCircle2,
    MessageCircle,
    Save,
    ToggleLeft,
    ToggleRight
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as thinclaw from '../../lib/thinclaw';
import { toast } from 'sonner';
import { ThinClawModeBadge, useThinClawStatusSnapshot } from './ThinClawModeBadge';

// ── Channel icon mapping ────────────────────────────────────────
const CHANNEL_ICONS: Record<string, any> = {
    slack: MessageSquare,
    telegram: Send,
    discord: Hash,
    signal: Shield,
    webhook: Globe,
    nostr: Radio,
    whatsapp: Smartphone,
    gmail: Mail,
    apple_mail: Mail,
    imessage: MessageCircle,
    bluebubbles: Smartphone,
};

const CHANNEL_DESCRIPTIONS: Record<string, string> = {
    slack: 'Enterprise workspace bridge via Socket Mode. Supports streaming draft replies.',
    telegram: 'Full Telegram Bot API integration with forum topics, channel posts, and DM pairing.',
    discord: 'Native Rust gateway with WebSocket + REST. Streaming draft replies support.',
    signal: 'Encrypted messaging via signal-cli daemon with SSE listener.',
    webhook: 'HTTP webhook endpoint with HMAC-SHA256 signature verification. Always active.',
    nostr: 'NIP-04 encrypted direct messages on the Nostr protocol.',
    whatsapp: 'Bridge to WhatsApp via web authentication. Supports media and group chats.',
    gmail: 'Gmail integration via OAuth 2.0. Read and respond to email with label-based filtering.',
    apple_mail: 'Apple Mail integration (macOS only). Reads the local Envelope Index and sends via Mail.app.',
    imessage: 'iMessage channel (macOS only). Polls chat.db for incoming messages and responds via AppleScript.',
    bluebubbles: 'Cross-platform iMessage bridge via BlueBubbles server. Supports media, read receipts, and group chats.',
};

// Runtime stream-mode vocabulary (channels-core StreamMode::from_str_value):
// '' = None (send the full reply), 'edit' = live message edits, 'status' =
// typing-indicator, 'chunks' = event-chunked. Only telegram + discord honor it.
const STREAM_MODES = ['', 'edit', 'status', 'chunks'];
const STREAM_MODE_LABELS: Record<string, string> = {
    '': 'Off (full reply)',
    'edit': 'Live Edit',
    'status': 'Typing Indicator',
    'chunks': 'Chunked',
};

// ── Channel Card  ────────────────────────────────────────────────
interface ChannelCardProps {
    channel: thinclaw.ChannelInfo;
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
                "rounded-2xl border bg-card/30 backdrop-blur-md shadow-xs transition-all",
                channel.enabled
                    ? "border-primary/20 shadow-primary/5"
                    : "border-border/40"
            )}
        >
            <div className="p-6">
                <div className="flex items-start justify-between mb-4">
                    <div className="flex items-center gap-3">
                        <div className={cn(
                            "p-2.5 rounded-xl border",
                            channel.enabled
                                ? "bg-primary/10 border-primary/20"
                                : "bg-white/5 border-border/40"
                        )}>
                            <Icon className={cn("w-5 h-5", channel.enabled ? "text-primary" : "text-muted-foreground")} />
                        </div>
                        <div>
                            <h3 className="font-semibold">{channel.name}</h3>
                            <span className={cn(
                                "text-[10px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded",
                                channel.type === 'native' ? "bg-purple-500/10 text-primary" :
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
                                : "text-muted-foreground bg-white/5 border-border/40"
                        )}>
                            {channel.enabled ? 'Active' : 'Inactive'}
                        </div>
                    </div>
                </div>

                <p className="text-sm text-muted-foreground leading-relaxed">{description}</p>

                {/* Stream mode badge */}
                {hasStreamMode && channel.stream_mode && (
                    <div className="mt-3 flex items-center gap-2">
                        <Podcast className="w-3.5 h-3.5 text-muted-foreground" />
                        <span className="text-xs text-muted-foreground/80 font-medium">
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
                                                : "bg-white/3 text-muted-foreground hover:bg-white/5 border-white/5"
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
export function ThinClawChannels() {
    const [channels, setChannels] = useState<thinclaw.ChannelInfo[]>([]);
    const [isLoading, setIsLoading] = useState(true);
    const [expandedChannel, setExpandedChannel] = useState<string | null>(null);
    const [settingsMap, setSettingsMap] = useState<Record<string, any>>({});
    const { status: runtimeStatus, isRemote } = useThinClawStatusSnapshot(15000);
    const [appleAllowFrom, setAppleAllowFrom] = useState('');
    const [applePollInterval, setApplePollInterval] = useState('10');
    const [gmailEnabled, setGmailEnabled] = useState(false);
    const [gmailProjectId, setGmailProjectId] = useState('');
    const [gmailSubscriptionId, setGmailSubscriptionId] = useState('');
    const [gmailTopicId, setGmailTopicId] = useState('');
    const [gmailAllowedSenders, setGmailAllowedSenders] = useState('');

    const fetchChannels = useCallback(async () => {
        try {
            const resp = await thinclaw.getThinClawChannelsList();
            setChannels(resp.channels || []);
        } catch (e) {
            console.error('Failed to fetch channels:', e);
            // Fallback: use status API
            try {
                const status = await thinclaw.getThinClawStatus();
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

    const fetchSettings = useCallback(async () => {
        try {
            const resp = await thinclaw.listSettings();
            const next: Record<string, any> = {};
            for (const item of resp.settings || []) {
                next[item.key] = item.value;
            }
            setSettingsMap(next);
        } catch (e) {
            console.error('Failed to fetch channel settings:', e);
        }
    }, []);

    useEffect(() => {
        fetchChannels();
        fetchSettings();
    }, [fetchChannels, fetchSettings]);

    // ── Gmail OAuth + Status ────────────────────────────────────────────
    const [gmailConnecting, setGmailConnecting] = useState(false);
    const [gmailSaving, setGmailSaving] = useState(false);
    const [gmailConnected, setGmailConnected] = useState(false);
    const [gmailLabelFilter, setGmailLabelFilter] = useState('');
    const [gmailStatus, setGmailStatus] = useState<thinclaw.GmailStatusResponse | null>(null);

    const refreshGmailStatus = useCallback(async () => {
        const status = await thinclaw.getGmailStatus();
        setGmailStatus(status);
        setGmailConnected(status.oauth_configured);
        setGmailEnabled(status.enabled);
        setGmailProjectId(status.project_id);
        setGmailSubscriptionId(status.subscription_id);
        setGmailTopicId(status.topic_id);
        setGmailAllowedSenders(status.allowed_senders.join(', '));
        setGmailLabelFilter(status.label_filters.join(', '));
    }, []);

    // Load real Gmail status on mount.
    useEffect(() => {
        refreshGmailStatus().catch(() => {
            // Gmail status not available — leave editable defaults.
        });
    }, [refreshGmailStatus]);

    useEffect(() => {
        const allow = settingsMap['channels.apple_mail_allow_from'];
        setAppleAllowFrom(typeof allow === 'string' ? allow : '');
        const poll = settingsMap['channels.apple_mail_poll_interval'];
        setApplePollInterval(poll == null ? '10' : String(poll));
        const gmailSenders = settingsMap['channels.gmail_allowed_senders'];
        if (typeof gmailSenders === 'string') {
            setGmailAllowedSenders(gmailSenders);
        }
        const gmailLabels = settingsMap['channels.gmail_label_filters'];
        if (typeof gmailLabels === 'string') {
            setGmailLabelFilter(gmailLabels);
        }
        const gmailProject = settingsMap['channels.gmail_project_id'];
        if (typeof gmailProject === 'string') {
            setGmailProjectId(gmailProject);
        }
        const gmailSubscription = settingsMap['channels.gmail_subscription_id'];
        if (typeof gmailSubscription === 'string') {
            setGmailSubscriptionId(gmailSubscription);
        }
        const gmailTopic = settingsMap['channels.gmail_topic_id'];
        if (typeof gmailTopic === 'string') {
            setGmailTopicId(gmailTopic);
        }
        const gmailEnabledValue = settingsMap['channels.gmail_enabled'];
        if (typeof gmailEnabledValue === 'boolean') {
            setGmailEnabled(gmailEnabledValue);
        } else if (typeof gmailEnabledValue === 'string') {
            setGmailEnabled(gmailEnabledValue === 'true');
        }
    }, [settingsMap]);

    const settingBool = (key: string, fallback = false) => {
        const value = settingsMap[key];
        if (typeof value === 'boolean') return value;
        if (typeof value === 'string') return value === 'true';
        return fallback;
    };

    const saveSetting = async (key: string, value: any, label: string) => {
        try {
            await thinclaw.setSetting(key, value);
            setSettingsMap(prev => ({ ...prev, [key]: value }));
            toast.success(`${label} updated`);
            fetchChannels();
        } catch (e) {
            toast.error(`Failed to update ${label}: ${String(e)}`);
        }
    };

    const handleGmailConnect = async () => {
        setGmailConnecting(true);
        try {
            // Use ThinClaw's PKCE flow — opens browser, binds callback, exchanges tokens automatically
            const result = await thinclaw.startGmailOAuth();
            if (result.success) {
                setGmailConnected(true);
                // Refresh status after successful connection
                try {
                    await refreshGmailStatus();
                    await fetchChannels();
                } catch { /* ignore */ }
                toast.success('Gmail connected successfully!');
            } else {
                toast.error(result.error ?? 'Gmail connection failed');
            }
        } catch (e) {
            toast.error(`Gmail sign-in failed: ${String(e)}`);
        } finally {
            setGmailConnecting(false);
        }
    };

    const handleGmailSave = async () => {
        const projectId = gmailProjectId.trim();
        const subscriptionId = gmailSubscriptionId.trim();
        const topicId = gmailTopicId.trim();
        if (gmailEnabled && (!projectId || !subscriptionId || !topicId)) {
            toast.error('Project ID, subscription ID, and topic ID are required before enabling Gmail.');
            return;
        }

        const patch = {
            'channels.gmail_enabled': gmailEnabled,
            'channels.gmail_project_id': projectId,
            'channels.gmail_subscription_id': subscriptionId,
            'channels.gmail_topic_id': topicId,
            'channels.gmail_allowed_senders': gmailAllowedSenders.trim(),
            'channels.gmail_label_filters': gmailLabelFilter.trim(),
        };
        setGmailSaving(true);
        try {
            await thinclaw.patchSettings(patch);
            setSettingsMap(previous => ({ ...previous, ...patch }));
            await Promise.all([fetchChannels(), fetchSettings(), refreshGmailStatus()]);
            toast.success('Gmail settings applied');
        } catch (error) {
            toast.error(`Failed to apply Gmail settings: ${String(error)}`);
        } finally {
            setGmailSaving(false);
        }
    };

    const handleStreamModeChange = async (channelId: string, mode: string) => {
        // Runtime reads the namespaced per-channel key (telegram/discord only).
        const settingKey = `channels.${channelId}_stream_mode`;
        try {
            await thinclaw.setSetting(settingKey, mode);
            // Update local state
            setChannels(prev => prev.map(ch =>
                ch.id === channelId ? { ...ch, stream_mode: mode } : ch
            ));
            toast.success(`${channelId} stream mode set to ${STREAM_MODE_LABELS[mode] || mode}`);
        } catch (e) {
            toast.error(`Failed to update stream mode: ${e}`);
        }
    };

    const streamChannels = ['discord', 'telegram'];
    const activeChannels = channels.filter(ch => ch.enabled);
    const inactiveChannels = channels.filter(ch => !ch.enabled);
    const gmailRuntimeActive = channels.some(ch => ch.id === 'gmail' && ch.enabled);

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
                <div className="flex items-center gap-2">
                    <ThinClawModeBadge status={runtimeStatus} />
                    <button
                        onClick={() => { setIsLoading(true); fetchChannels(); fetchSettings(); }}
                        className="p-2.5 rounded-lg bg-card border border-border/40 hover:bg-white/5 transition-colors"
                    >
                        <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                    </button>
                </div>
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

            {/* Info Notice */}
            <div className="p-6 rounded-2xl border bg-blue-500/5 border-blue-500/20 flex gap-4">
                <div className="p-2 bg-blue-500/10 rounded-xl h-fit">
                    <Shield className="w-5 h-5 text-blue-500" />
                </div>
                <div>
                    <h4 className="text-sm font-semibold text-blue-500 uppercase tracking-wider">Channel Security</h4>
                    <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                        Communications are proxied through the ThinClaw Gateway.
                        Configure channels via environment variables or the config editor.
                        Streaming draft replies are supported on Discord, Telegram, and Slack.
                    </p>
                </div>
            </div>

            {/* Gmail Channel Card */}
            <div className="space-y-4">
                <h2 className="text-xs font-bold uppercase tracking-widest text-muted-foreground flex items-center gap-2">
                    <Mail className="w-3.5 h-3.5 text-red-400" />
                    Email Channels
                </h2>
                <motion.div
                    initial={{ opacity: 0, y: 5 }}
                    animate={{ opacity: 1, y: 0 }}
                    className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md overflow-hidden"
                >
                    <div className="p-5">
                        <div className="flex items-start gap-4">
                            <div className={cn(
                                "p-3 rounded-xl border transition-colors",
                                gmailRuntimeActive
                                    ? "bg-red-500/10 border-red-500/20 text-red-400"
                                    : "bg-white/5 border-border/40 text-muted-foreground"
                            )}>
                                <Mail className="w-5 h-5" />
                            </div>
                            <div className="flex-1 min-w-0">
                                <div className="flex items-center gap-2">
                                    <h3 className="font-semibold text-sm">Gmail</h3>
                                    <span className={cn(
                                        "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium border",
                                        gmailRuntimeActive
                                            ? "text-primary bg-emerald-500/10 border-emerald-500/20"
                                            : "text-muted-foreground bg-zinc-500/10 border-zinc-500/20"
                                    )}>
                                        {gmailRuntimeActive ? (
                                            <><CheckCircle2 className="w-3 h-3" /> Active</>
                                        ) : gmailConnected ? (
                                            <><CheckCircle2 className="w-3 h-3" /> Authorized</>
                                        ) : (
                                            'Not authorized'
                                        )}
                                    </span>
                                </div>
                                <p className="text-xs text-muted-foreground mt-1.5 leading-relaxed">
                                    {CHANNEL_DESCRIPTIONS['gmail']}
                                </p>

                                <div className="mt-3 space-y-4">
                                    {gmailStatus && (
                                        <div className="flex flex-wrap items-center gap-2">
                                            <span className={cn(
                                                "inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium",
                                                gmailRuntimeActive
                                                    ? "bg-emerald-500/10 text-primary"
                                                    : "bg-amber-500/10 text-muted-foreground"
                                            )}>
                                                {gmailRuntimeActive ? 'Runtime active' : gmailStatus.status}
                                            </span>
                                            {gmailConnected && !gmailRuntimeActive && (
                                                <span className="text-[10px] text-muted-foreground">
                                                    OAuth is authorized, but the channel is not running.
                                                </span>
                                            )}
                                        </div>
                                    )}

                                    <button
                                        type="button"
                                        onClick={() => setGmailEnabled(value => !value)}
                                        className="flex w-full items-center justify-between rounded-xl border border-border/40 bg-white/3 px-3 py-2 text-left"
                                    >
                                        <span>
                                            <span className="block text-xs font-medium">Enable Gmail channel</span>
                                            <span className="block text-[10px] text-muted-foreground">
                                                Starts the Pub/Sub subscriber after settings are applied.
                                            </span>
                                        </span>
                                        {gmailEnabled
                                            ? <ToggleRight className="h-6 w-6 text-red-400" />
                                            : <ToggleLeft className="h-6 w-6 text-muted-foreground" />}
                                    </button>

                                    <div className="grid grid-cols-1 gap-3 md:grid-cols-3">
                                        {[
                                            ['Google Cloud Project ID', gmailProjectId, setGmailProjectId, 'my-project'],
                                            ['Pub/Sub Subscription ID', gmailSubscriptionId, setGmailSubscriptionId, 'gmail-events-sub'],
                                            ['Pub/Sub Topic ID', gmailTopicId, setGmailTopicId, 'gmail-events'],
                                        ].map(([label, value, setter, placeholder]) => (
                                            <label key={label as string} className="space-y-1">
                                                <span className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                                                    {label as string}
                                                </span>
                                                <input
                                                    type="text"
                                                    value={value as string}
                                                    onChange={event => (setter as (value: string) => void)(event.target.value)}
                                                    placeholder={placeholder as string}
                                                    className="h-8 w-full rounded-lg border border-border/40 bg-white/3 px-3 text-xs font-mono outline-hidden transition-all focus:ring-1 focus:ring-primary/30"
                                                />
                                            </label>
                                        ))}
                                    </div>

                                    <div>
                                        <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                                            Allowed Senders
                                        </label>
                                        <input
                                            type="text"
                                            value={gmailAllowedSenders}
                                            onChange={event => setGmailAllowedSenders(event.target.value)}
                                            placeholder="name@example.com, @example.com"
                                            className="mt-1 h-8 w-full rounded-lg border border-border/40 bg-white/3 px-3 text-xs font-mono outline-hidden transition-all focus:ring-1 focus:ring-primary/30"
                                        />
                                        <p className="mt-0.5 text-[10px] text-amber-500/80">
                                            Empty means every sender is allowed. Use comma-separated addresses or @domain rules.
                                        </p>
                                    </div>

                                    <div>
                                        <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                                            Label Filters
                                        </label>
                                        <input
                                            type="text"
                                            value={gmailLabelFilter}
                                            onChange={event => setGmailLabelFilter(event.target.value)}
                                            placeholder="INBOX, UNREAD"
                                            className="mt-1 h-8 w-full rounded-lg border border-border/40 bg-white/3 px-3 text-xs font-mono outline-hidden transition-all focus:ring-1 focus:ring-primary/30"
                                        />
                                        <p className="mt-0.5 text-[10px] text-muted-foreground/50">
                                            Empty watches unread mail from the bounded recent-history window.
                                        </p>
                                    </div>

                                    {isRemote && (
                                        <p className="text-[10px] text-amber-500/80">
                                            OAuth must be completed in a desktop session on the gateway host.
                                        </p>
                                    )}
                                    <div className="flex flex-wrap gap-2">
                                        <button
                                            type="button"
                                            onClick={handleGmailConnect}
                                            disabled={gmailConnecting || isRemote}
                                            className={cn(
                                                "flex items-center gap-2 rounded-xl border border-red-500/20 bg-red-500/10 px-4 py-2 text-xs font-bold uppercase tracking-wider text-red-400 transition-all hover:bg-red-500/20",
                                                (gmailConnecting || isRemote) && "cursor-not-allowed opacity-50"
                                            )}
                                        >
                                            {gmailConnecting ? (
                                                <><Loader2 className="h-3.5 w-3.5 animate-spin" /> Connecting…</>
                                            ) : (
                                                <><ExternalLink className="h-3.5 w-3.5" /> {gmailConnected ? 'Reauthorize' : 'Authorize Gmail'}</>
                                            )}
                                        </button>
                                        <button
                                            type="button"
                                            onClick={handleGmailSave}
                                            disabled={gmailSaving}
                                            className={cn(
                                                "flex items-center gap-2 rounded-xl border border-primary/20 bg-primary/10 px-4 py-2 text-xs font-bold uppercase tracking-wider text-primary transition-all hover:bg-primary/20",
                                                gmailSaving && "cursor-wait opacity-50"
                                            )}
                                        >
                                            {gmailSaving
                                                ? <Loader2 className="h-3.5 w-3.5 animate-spin" />
                                                : <Save className="h-3.5 w-3.5" />}
                                            Apply Gmail Settings
                                        </button>
                                    </div>
                                </div>
                            </div>
                        </div>
                    </div>
                </motion.div>

                <motion.div
                    initial={{ opacity: 0, y: 5 }}
                    animate={{ opacity: 1, y: 0 }}
                    className="rounded-2xl border border-border/40 bg-card/30 backdrop-blur-md overflow-hidden"
                >
                    <div className="p-5">
                        <div className="flex items-start gap-4">
                            <div className={cn(
                                "p-3 rounded-xl border transition-colors",
                                settingBool('channels.apple_mail_enabled')
                                    ? "bg-blue-500/10 border-blue-500/20 text-blue-400"
                                    : "bg-white/5 border-border/40 text-muted-foreground"
                            )}>
                                <Mail className="w-5 h-5" />
                            </div>
                            <div className="flex-1 min-w-0">
                                <div className="flex flex-wrap items-center gap-2">
                                    <h3 className="font-semibold text-sm">Apple Mail</h3>
                                    <span className={cn(
                                        "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium border",
                                        settingBool('channels.apple_mail_enabled')
                                            ? "text-blue-400 bg-blue-500/10 border-blue-500/20"
                                            : "text-muted-foreground bg-zinc-500/10 border-zinc-500/20"
                                    )}>
                                        {settingBool('channels.apple_mail_enabled') ? 'Enabled' : 'Disabled'}
                                    </span>
                                    {isRemote && (
                                        <span className="rounded-full border border-cyan-500/20 bg-cyan-500/10 px-2 py-0.5 text-[10px] font-medium text-cyan-400">
                                            Host-side macOS access
                                        </span>
                                    )}
                                </div>
                                <p className="text-xs text-muted-foreground mt-1.5 leading-relaxed">
                                    {CHANNEL_DESCRIPTIONS['apple_mail']}
                                </p>

                                <div className="mt-4 grid gap-3 md:grid-cols-2">
                                    <button
                                        onClick={() => saveSetting('channels.apple_mail_enabled', !settingBool('channels.apple_mail_enabled'), 'Apple Mail')}
                                        className={cn(
                                            "flex items-center justify-between rounded-xl border px-3 py-2 text-xs font-medium transition-all",
                                            settingBool('channels.apple_mail_enabled')
                                                ? "border-blue-500/25 bg-blue-500/10 text-blue-400"
                                                : "border-border/40 bg-white/3 text-muted-foreground hover:text-foreground"
                                        )}
                                    >
                                        <span>Enabled</span>
                                        {settingBool('channels.apple_mail_enabled') ? <ToggleRight className="h-4 w-4" /> : <ToggleLeft className="h-4 w-4" />}
                                    </button>
                                    <div className="flex gap-2">
                                        <input
                                            type="number"
                                            min={5}
                                            max={120}
                                            value={applePollInterval}
                                            onChange={e => setApplePollInterval(e.target.value)}
                                            className="h-9 min-w-0 flex-1 rounded-lg border border-border/40 bg-white/3 px-3 text-xs font-mono outline-hidden focus:ring-1 focus:ring-primary/30"
                                        />
                                        <button
                                            onClick={() => saveSetting('channels.apple_mail_poll_interval', Number(applePollInterval) || 10, 'Apple Mail poll interval')}
                                            className="rounded-lg border border-border/40 bg-white/3 px-3 text-[10px] font-bold uppercase tracking-wider text-muted-foreground hover:text-foreground"
                                        >
                                            Save
                                        </button>
                                    </div>
                                    <div className="md:col-span-2">
                                        <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                                            Allowed Senders
                                        </label>
                                        <div className="mt-1 flex gap-2">
                                            <input
                                                type="text"
                                                value={appleAllowFrom}
                                                onChange={e => setAppleAllowFrom(e.target.value)}
                                                placeholder="name@example.com, team@example.com"
                                                className="h-8 min-w-0 flex-1 rounded-lg border border-border/40 bg-white/3 px-3 text-xs font-mono outline-hidden transition-all focus:ring-1 focus:ring-primary/30"
                                            />
                                            <button
                                                onClick={() => saveSetting('channels.apple_mail_allow_from', appleAllowFrom.trim() || null, 'Apple Mail senders')}
                                                className="flex h-8 items-center gap-1.5 rounded-lg border border-border/40 bg-white/3 px-3 text-[10px] font-bold uppercase tracking-wider text-muted-foreground hover:text-foreground"
                                            >
                                                <Save className="h-3.5 w-3.5" />
                                                Save
                                            </button>
                                        </div>
                                    </div>
                                    <button
                                        onClick={() => saveSetting('channels.apple_mail_unread_only', !settingBool('channels.apple_mail_unread_only', true), 'Unread-only mode')}
                                        className="flex items-center justify-between rounded-xl border border-border/40 bg-white/3 px-3 py-2 text-xs text-muted-foreground hover:text-foreground"
                                    >
                                        <span>Unread only</span>
                                        {settingBool('channels.apple_mail_unread_only', true) ? <ToggleRight className="h-4 w-4 text-blue-400" /> : <ToggleLeft className="h-4 w-4" />}
                                    </button>
                                    <button
                                        onClick={() => saveSetting('channels.apple_mail_mark_as_read', !settingBool('channels.apple_mail_mark_as_read', true), 'Mark-as-read mode')}
                                        className="flex items-center justify-between rounded-xl border border-border/40 bg-white/3 px-3 py-2 text-xs text-muted-foreground hover:text-foreground"
                                    >
                                        <span>Mark as read</span>
                                        {settingBool('channels.apple_mail_mark_as_read', true) ? <ToggleRight className="h-4 w-4 text-blue-400" /> : <ToggleLeft className="h-4 w-4" />}
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>
                </motion.div>
            </div>
        </motion.div>
    );
}
