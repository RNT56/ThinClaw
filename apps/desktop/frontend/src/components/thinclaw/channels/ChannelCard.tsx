import { AnimatePresence, motion } from 'framer-motion';
import { ChevronDown, ChevronUp, Podcast, Settings2 } from 'lucide-react';
import type { ChannelInfo } from '../../../lib/thinclaw';
import { cn } from '../../../lib/utils';
import { CHANNEL_DESCRIPTIONS, STREAM_MODE_LABELS, STREAM_MODES, channelIcon } from './catalog';

interface ChannelCardProps {
    channel: ChannelInfo;
    onConfigureStream?: (id: string, mode: string) => void;
    hasStreamMode: boolean;
    expanded: boolean;
    onToggleExpand: () => void;
}

export function ChannelCard({ channel, onConfigureStream, hasStreamMode, expanded, onToggleExpand }: ChannelCardProps) {
    const Icon = channelIcon(channel.id);
    const description = CHANNEL_DESCRIPTIONS[channel.id] || `${channel.name} channel (${channel.type})`;

    return (
        <motion.div
            layout
            className={cn(
                'rounded-2xl border bg-card/30 backdrop-blur-md shadow-xs transition-all',
                channel.enabled ? 'border-primary/20 shadow-primary/5' : 'border-border/40'
            )}
        >
            <div className="p-6">
                <div className="flex items-start justify-between mb-4">
                    <div className="flex items-center gap-3">
                        <div
                            className={cn(
                                'p-2.5 rounded-xl border',
                                channel.enabled ? 'bg-primary/10 border-primary/20' : 'bg-white/5 border-border/40'
                            )}
                        >
                            <Icon
                                className={cn('w-5 h-5', channel.enabled ? 'text-primary' : 'text-muted-foreground')}
                            />
                        </div>
                        <div>
                            <h3 className="font-semibold">{channel.name}</h3>
                            <span
                                className={cn(
                                    'text-[10px] font-bold uppercase tracking-wider px-1.5 py-0.5 rounded',
                                    channel.type === 'native'
                                        ? 'bg-purple-500/10 text-primary'
                                        : channel.type === 'wasm'
                                          ? 'bg-blue-500/10 text-blue-400'
                                          : 'bg-green-500/10 text-green-400'
                                )}
                            >
                                {channel.type}
                            </span>
                        </div>
                    </div>
                    <div
                        className={cn(
                            'px-2.5 py-1 rounded-full text-[10px] font-bold uppercase tracking-wider border',
                            channel.enabled
                                ? 'text-green-500 bg-green-500/10 border-green-500/20'
                                : 'text-muted-foreground bg-white/5 border-border/40'
                        )}
                    >
                        {channel.enabled ? 'Active' : 'Inactive'}
                    </div>
                </div>

                <p className="text-sm text-muted-foreground leading-relaxed">{description}</p>
                {hasStreamMode && channel.stream_mode && (
                    <div className="mt-3 flex items-center gap-2">
                        <Podcast className="w-3.5 h-3.5 text-muted-foreground" />
                        <span className="text-xs text-muted-foreground/80 font-medium">
                            Stream: {STREAM_MODE_LABELS[channel.stream_mode] || channel.stream_mode}
                        </span>
                    </div>
                )}
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

            <AnimatePresence>
                {expanded && hasStreamMode && (
                    <motion.div
                        initial={{ height: 0, opacity: 0 }}
                        animate={{ height: 'auto', opacity: 1 }}
                        exit={{ height: 0, opacity: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="px-6 pb-6 pt-2 border-t border-white/5 space-y-3">
                            <p className="text-[10px] uppercase font-bold text-muted-foreground tracking-widest">
                                Streaming Mode
                            </p>
                            <div className="grid grid-cols-2 gap-2">
                                {STREAM_MODES.map((mode) => (
                                    <button
                                        key={mode}
                                        onClick={() => onConfigureStream?.(channel.id, mode)}
                                        className={cn(
                                            'px-3 py-2 rounded-lg text-xs font-medium transition-all border',
                                            (channel.stream_mode || '') === mode
                                                ? 'bg-primary/15 text-primary border-primary/30'
                                                : 'bg-white/3 text-muted-foreground hover:bg-white/5 border-white/5'
                                        )}
                                    >
                                        {STREAM_MODE_LABELS[mode]}
                                    </button>
                                ))}
                            </div>
                            <p className="text-[10px] text-muted-foreground/60 leading-relaxed">
                                Controls how the agent streams partial replies in this channel. &quot;Live Edit&quot;
                                updates one reply, while &quot;Typing Indicator&quot; only reports activity.
                            </p>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}
