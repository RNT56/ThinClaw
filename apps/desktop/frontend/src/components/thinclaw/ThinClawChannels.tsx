import { motion } from 'framer-motion';
import {
    RefreshCw,
    Shield,
    Zap,
    Mail,
    ExternalLink,
    Loader2,
    CheckCircle2,
    Save,
    ToggleLeft,
    ToggleRight
} from 'lucide-react';
import { cn } from '../../lib/utils';
import { ThinClawModeBadge } from './ThinClawModeBadge';
import { ChannelCard } from './channels/ChannelCard';
import { CHANNEL_DESCRIPTIONS } from './channels/catalog';
import { useChannels } from './channels/use-channels';

// ── Main Component ──────────────────────────────────────────────
export function ThinClawChannels() {
    const {
        channels,
        isLoading,
        setIsLoading,
        expandedChannel,
        setExpandedChannel,
        runtimeStatus,
        isRemote,
        appleAllowFrom,
        setAppleAllowFrom,
        applePollInterval,
        setApplePollInterval,
        gmailAllowedSenders,
        setGmailAllowedSenders,
        gmailConnecting,
        gmailConnected,
        gmailLabelFilter,
        setGmailLabelFilter,
        gmailStatus,
        fetchChannels,
        fetchSettings,
        settingBool,
        saveSetting,
        handleGmailConnect,
        handleStreamModeChange
    } = useChannels();

    const streamChannels = ['discord', 'telegram'];
    const activeChannels = channels.filter((ch) => ch.enabled);
    const inactiveChannels = channels.filter((ch) => !ch.enabled);

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
                        onClick={() => {
                            setIsLoading(true);
                            fetchChannels();
                            fetchSettings();
                        }}
                        className="p-2.5 rounded-lg bg-card border border-border/40 hover:bg-white/5 transition-colors"
                    >
                        <RefreshCw className={cn('w-4 h-4', isLoading && 'animate-spin')} />
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
                        {activeChannels.map((ch) => (
                            <ChannelCard
                                key={ch.id}
                                channel={ch}
                                hasStreamMode={streamChannels.includes(ch.id)}
                                expanded={expandedChannel === ch.id}
                                onToggleExpand={() => setExpandedChannel((prev) => (prev === ch.id ? null : ch.id))}
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
                        {inactiveChannels.map((ch) => (
                            <ChannelCard
                                key={ch.id}
                                channel={ch}
                                hasStreamMode={streamChannels.includes(ch.id)}
                                expanded={expandedChannel === ch.id}
                                onToggleExpand={() => setExpandedChannel((prev) => (prev === ch.id ? null : ch.id))}
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
                        Communications are proxied through the ThinClaw Gateway. Configure channels via environment
                        variables or the config editor. Streaming draft replies are supported on Discord, Telegram, and
                        Slack.
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
                            <div
                                className={cn(
                                    'p-3 rounded-xl border transition-colors',
                                    gmailConnected
                                        ? 'bg-red-500/10 border-red-500/20 text-red-400'
                                        : 'bg-white/5 border-border/40 text-muted-foreground'
                                )}
                            >
                                <Mail className="w-5 h-5" />
                            </div>
                            <div className="flex-1 min-w-0">
                                <div className="flex items-center gap-2">
                                    <h3 className="font-semibold text-sm">Gmail</h3>
                                    <span
                                        className={cn(
                                            'inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium border',
                                            gmailConnected
                                                ? 'text-primary bg-emerald-500/10 border-emerald-500/20'
                                                : 'text-muted-foreground bg-zinc-500/10 border-zinc-500/20'
                                        )}
                                    >
                                        {gmailConnected ? (
                                            <>
                                                <CheckCircle2 className="w-3 h-3" /> Connected
                                            </>
                                        ) : (
                                            'Not configured'
                                        )}
                                    </span>
                                </div>
                                <p className="text-xs text-muted-foreground mt-1.5 leading-relaxed">
                                    {CHANNEL_DESCRIPTIONS['gmail']}
                                </p>

                                {gmailConnected ? (
                                    <div className="mt-3 space-y-3">
                                        {/* Status indicator */}
                                        {gmailStatus && (
                                            <div className="flex items-center gap-2">
                                                <span
                                                    className={cn(
                                                        'inline-flex items-center gap-1 px-2 py-0.5 rounded text-[10px] font-medium',
                                                        gmailStatus.status.startsWith('ready')
                                                            ? 'bg-emerald-500/10 text-primary'
                                                            : 'bg-amber-500/10 text-muted-foreground'
                                                    )}
                                                >
                                                    {gmailStatus.status}
                                                </span>
                                            </div>
                                        )}
                                        <div>
                                            <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                                                Allowed Senders
                                            </label>
                                            <div className="mt-1 flex gap-2">
                                                <input
                                                    type="text"
                                                    value={gmailAllowedSenders}
                                                    onChange={(e) => setGmailAllowedSenders(e.target.value)}
                                                    placeholder="name@example.com, team@example.com"
                                                    className="h-8 min-w-0 flex-1 rounded-lg border border-border/40 bg-white/3 px-3 text-xs font-mono outline-hidden transition-all focus:ring-1 focus:ring-primary/30"
                                                />
                                                <button
                                                    onClick={() =>
                                                        saveSetting(
                                                            'channels.gmail_allowed_senders',
                                                            gmailAllowedSenders.trim() || null,
                                                            'Gmail senders'
                                                        )
                                                    }
                                                    className="flex h-8 items-center gap-1.5 rounded-lg border border-red-500/20 bg-red-500/10 px-3 text-[10px] font-bold uppercase tracking-wider text-red-400 hover:bg-red-500/20"
                                                >
                                                    <Save className="h-3.5 w-3.5" />
                                                    Save
                                                </button>
                                            </div>
                                        </div>
                                        {/* Label filter */}
                                        <div>
                                            <label className="text-[10px] font-bold uppercase tracking-widest text-muted-foreground/60">
                                                Label Filter
                                            </label>
                                            <div className="mt-1 flex items-center gap-2">
                                                <input
                                                    type="text"
                                                    value={gmailLabelFilter}
                                                    onChange={(e) => setGmailLabelFilter(e.target.value)}
                                                    placeholder="INBOX, Category:Primary"
                                                    className="h-8 min-w-0 flex-1 rounded-lg border border-border/40 bg-white/3 px-3 text-xs font-mono focus:ring-1 focus:ring-primary/30 outline-hidden transition-all"
                                                />
                                                <button
                                                    onClick={() =>
                                                        saveSetting(
                                                            'channels.gmail_label_filters',
                                                            gmailLabelFilter.trim() || null,
                                                            'Gmail label filter'
                                                        )
                                                    }
                                                    className="flex h-8 items-center gap-1.5 rounded-lg border border-red-500/20 bg-red-500/10 px-3 text-[10px] font-bold uppercase tracking-wider text-red-400 hover:bg-red-500/20"
                                                >
                                                    <Save className="h-3.5 w-3.5" />
                                                    Save
                                                </button>
                                            </div>
                                            <p className="text-[10px] text-muted-foreground/50 mt-0.5">
                                                Comma-separated Gmail labels to watch. Leave empty to watch INBOX.
                                            </p>
                                        </div>
                                    </div>
                                ) : (
                                    <button
                                        onClick={handleGmailConnect}
                                        disabled={gmailConnecting}
                                        className={cn(
                                            'mt-3 flex items-center gap-2 px-4 py-2 rounded-xl text-xs font-bold uppercase tracking-wider',
                                            'bg-red-500/10 text-red-400 border border-red-500/20',
                                            'hover:bg-red-500/20 transition-all',
                                            gmailConnecting && 'opacity-50 cursor-wait'
                                        )}
                                    >
                                        {gmailConnecting ? (
                                            <>
                                                <Loader2 className="w-3.5 h-3.5 animate-spin" /> Connecting…
                                            </>
                                        ) : (
                                            <>
                                                <ExternalLink className="w-3.5 h-3.5" /> Connect Gmail
                                            </>
                                        )}
                                    </button>
                                )}
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
                            <div
                                className={cn(
                                    'p-3 rounded-xl border transition-colors',
                                    settingBool('channels.apple_mail_enabled')
                                        ? 'bg-blue-500/10 border-blue-500/20 text-blue-400'
                                        : 'bg-white/5 border-border/40 text-muted-foreground'
                                )}
                            >
                                <Mail className="w-5 h-5" />
                            </div>
                            <div className="flex-1 min-w-0">
                                <div className="flex flex-wrap items-center gap-2">
                                    <h3 className="font-semibold text-sm">Apple Mail</h3>
                                    <span
                                        className={cn(
                                            'inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium border',
                                            settingBool('channels.apple_mail_enabled')
                                                ? 'text-blue-400 bg-blue-500/10 border-blue-500/20'
                                                : 'text-muted-foreground bg-zinc-500/10 border-zinc-500/20'
                                        )}
                                    >
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
                                        onClick={() =>
                                            saveSetting(
                                                'channels.apple_mail_enabled',
                                                !settingBool('channels.apple_mail_enabled'),
                                                'Apple Mail'
                                            )
                                        }
                                        className={cn(
                                            'flex items-center justify-between rounded-xl border px-3 py-2 text-xs font-medium transition-all',
                                            settingBool('channels.apple_mail_enabled')
                                                ? 'border-blue-500/25 bg-blue-500/10 text-blue-400'
                                                : 'border-border/40 bg-white/3 text-muted-foreground hover:text-foreground'
                                        )}
                                    >
                                        <span>Enabled</span>
                                        {settingBool('channels.apple_mail_enabled') ? (
                                            <ToggleRight className="h-4 w-4" />
                                        ) : (
                                            <ToggleLeft className="h-4 w-4" />
                                        )}
                                    </button>
                                    <div className="flex gap-2">
                                        <input
                                            type="number"
                                            min={5}
                                            max={120}
                                            value={applePollInterval}
                                            onChange={(e) => setApplePollInterval(e.target.value)}
                                            className="h-9 min-w-0 flex-1 rounded-lg border border-border/40 bg-white/3 px-3 text-xs font-mono outline-hidden focus:ring-1 focus:ring-primary/30"
                                        />
                                        <button
                                            onClick={() =>
                                                saveSetting(
                                                    'channels.apple_mail_poll_interval',
                                                    Number(applePollInterval) || 10,
                                                    'Apple Mail poll interval'
                                                )
                                            }
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
                                                onChange={(e) => setAppleAllowFrom(e.target.value)}
                                                placeholder="name@example.com, team@example.com"
                                                className="h-8 min-w-0 flex-1 rounded-lg border border-border/40 bg-white/3 px-3 text-xs font-mono outline-hidden transition-all focus:ring-1 focus:ring-primary/30"
                                            />
                                            <button
                                                onClick={() =>
                                                    saveSetting(
                                                        'channels.apple_mail_allow_from',
                                                        appleAllowFrom.trim() || null,
                                                        'Apple Mail senders'
                                                    )
                                                }
                                                className="flex h-8 items-center gap-1.5 rounded-lg border border-border/40 bg-white/3 px-3 text-[10px] font-bold uppercase tracking-wider text-muted-foreground hover:text-foreground"
                                            >
                                                <Save className="h-3.5 w-3.5" />
                                                Save
                                            </button>
                                        </div>
                                    </div>
                                    <button
                                        onClick={() =>
                                            saveSetting(
                                                'channels.apple_mail_unread_only',
                                                !settingBool('channels.apple_mail_unread_only', true),
                                                'Unread-only mode'
                                            )
                                        }
                                        className="flex items-center justify-between rounded-xl border border-border/40 bg-white/3 px-3 py-2 text-xs text-muted-foreground hover:text-foreground"
                                    >
                                        <span>Unread only</span>
                                        {settingBool('channels.apple_mail_unread_only', true) ? (
                                            <ToggleRight className="h-4 w-4 text-blue-400" />
                                        ) : (
                                            <ToggleLeft className="h-4 w-4" />
                                        )}
                                    </button>
                                    <button
                                        onClick={() =>
                                            saveSetting(
                                                'channels.apple_mail_mark_as_read',
                                                !settingBool('channels.apple_mail_mark_as_read', true),
                                                'Mark-as-read mode'
                                            )
                                        }
                                        className="flex items-center justify-between rounded-xl border border-border/40 bg-white/3 px-3 py-2 text-xs text-muted-foreground hover:text-foreground"
                                    >
                                        <span>Mark as read</span>
                                        {settingBool('channels.apple_mail_mark_as_read', true) ? (
                                            <ToggleRight className="h-4 w-4 text-blue-400" />
                                        ) : (
                                            <ToggleLeft className="h-4 w-4" />
                                        )}
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
