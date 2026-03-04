import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
    RefreshCw,
    Anchor,
    Shield,
    Clock,
    ArrowUpDown,
    ChevronDown,
    ChevronRight,
    AlertCircle,
    Zap,
    Ban,
} from 'lucide-react';
import { cn } from '../../lib/utils';
import * as openclaw from '../../lib/openclaw';
import { toast } from 'sonner';

const HOOK_POINT_COLORS: Record<string, string> = {
    beforeInbound: 'bg-blue-500/15 text-blue-400 border-blue-500/20',
    beforeToolCall: 'bg-amber-500/15 text-amber-400 border-amber-500/20',
    beforeOutbound: 'bg-green-500/15 text-green-400 border-green-500/20',
    onSessionStart: 'bg-purple-500/15 text-purple-400 border-purple-500/20',
    onSessionEnd: 'bg-red-500/15 text-red-400 border-red-500/20',
    transformResponse: 'bg-cyan-500/15 text-cyan-400 border-cyan-500/20',
    beforeAgentStart: 'bg-orange-500/15 text-orange-400 border-orange-500/20',
    beforeMessageWrite: 'bg-pink-500/15 text-pink-400 border-pink-500/20',
};

const HOOK_POINT_ICONS: Record<string, React.ReactNode> = {
    beforeInbound: <Anchor className="w-3 h-3" />,
    beforeToolCall: <Zap className="w-3 h-3" />,
    beforeOutbound: <ArrowUpDown className="w-3 h-3" />,
    onSessionStart: <ChevronRight className="w-3 h-3" />,
    onSessionEnd: <Ban className="w-3 h-3" />,
    transformResponse: <RefreshCw className="w-3 h-3" />,
    beforeAgentStart: <Shield className="w-3 h-3" />,
    beforeMessageWrite: <ArrowUpDown className="w-3 h-3" />,
};

function HookCard({ hook }: { hook: openclaw.HookInfoItem }) {
    const [expanded, setExpanded] = useState(false);
    const isFailClosed = hook.failure_mode === 'FailClosed';

    return (
        <motion.div
            layout
            className={cn(
                "rounded-2xl border transition-all duration-300",
                "bg-white/[0.02] border-white/5 hover:border-white/10",
                "shadow-sm hover:shadow-md"
            )}
        >
            <button
                onClick={() => setExpanded(!expanded)}
                className="w-full p-5 flex items-start justify-between text-left"
            >
                <div className="flex items-center gap-3 flex-1 min-w-0">
                    <div className={cn(
                        "p-2.5 rounded-xl border transition-colors flex items-center justify-center",
                        "bg-primary/10 border-primary/20 text-primary"
                    )}>
                        <Anchor className="w-5 h-5" />
                    </div>
                    <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                            <h3 className="font-semibold text-sm truncate">{hook.name}</h3>
                            <span className="text-[10px] font-mono text-muted-foreground/60 px-1.5 py-0.5 rounded bg-white/5 border border-white/5">
                                P{hook.priority}
                            </span>
                            {isFailClosed && (
                                <span className="text-[9px] font-bold uppercase tracking-tight px-1.5 py-0.5 rounded bg-red-500/10 border border-red-500/20 text-red-400">
                                    Fail-Closed
                                </span>
                            )}
                        </div>
                        <div className="flex flex-wrap gap-1.5 mt-2">
                            {hook.hook_points.map(point => (
                                <span
                                    key={point}
                                    className={cn(
                                        "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-[10px] font-medium border",
                                        HOOK_POINT_COLORS[point] || 'bg-white/5 text-muted-foreground border-white/10'
                                    )}
                                >
                                    {HOOK_POINT_ICONS[point]}
                                    {point}
                                </span>
                            ))}
                        </div>
                    </div>
                </div>
                <ChevronDown className={cn(
                    "w-4 h-4 text-muted-foreground transition-transform flex-none mt-1",
                    expanded && "rotate-180"
                )} />
            </button>

            <AnimatePresence>
                {expanded && (
                    <motion.div
                        initial={{ opacity: 0, height: 0 }}
                        animate={{ opacity: 1, height: 'auto' }}
                        exit={{ opacity: 0, height: 0 }}
                        className="overflow-hidden"
                    >
                        <div className="px-5 pb-5 pt-0 border-t border-white/5">
                            <div className="mt-4 grid grid-cols-2 gap-3">
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1">
                                        <Clock className="w-3 h-3" />
                                        Timeout
                                    </div>
                                    <p className="text-sm font-mono font-medium">
                                        {hook.timeout_ms >= 1000
                                            ? `${(hook.timeout_ms / 1000).toFixed(1)}s`
                                            : `${hook.timeout_ms}ms`}
                                    </p>
                                </div>
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1">
                                        <Shield className="w-3 h-3" />
                                        Failure Mode
                                    </div>
                                    <p className={cn(
                                        "text-sm font-medium",
                                        isFailClosed ? "text-red-400" : "text-green-400"
                                    )}>
                                        {hook.failure_mode.replace(/([A-Z])/g, ' $1').trim()}
                                    </p>
                                </div>
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1">
                                        <ArrowUpDown className="w-3 h-3" />
                                        Priority
                                    </div>
                                    <p className="text-sm font-mono font-medium">
                                        {hook.priority}
                                        <span className="text-muted-foreground/50 text-xs ml-1">
                                            ({hook.priority < 50 ? 'high' : hook.priority < 150 ? 'normal' : 'low'})
                                        </span>
                                    </p>
                                </div>
                                <div className="p-3 rounded-lg bg-white/[0.03] border border-white/5">
                                    <div className="flex items-center gap-1.5 text-[10px] uppercase tracking-wider font-bold text-muted-foreground/60 mb-1">
                                        <Anchor className="w-3 h-3" />
                                        Hook Points
                                    </div>
                                    <p className="text-sm font-mono font-medium">
                                        {hook.hook_points.length}
                                    </p>
                                </div>
                            </div>
                        </div>
                    </motion.div>
                )}
            </AnimatePresence>
        </motion.div>
    );
}

export function OpenClawHooks() {
    const [hooks, setHooks] = useState<openclaw.HookInfoItem[]>([]);
    const [isLoading, setIsLoading] = useState(true);

    const fetchHooks = async () => {
        try {
            const data = await openclaw.listHooks();
            setHooks(data.hooks || []);
        } catch (e) {
            console.error('Failed to fetch hooks:', e);
            toast.error('Failed to load hooks');
        } finally {
            setIsLoading(false);
        }
    };

    useEffect(() => {
        fetchHooks();
    }, []);

    // Group hooks by hook point for the summary
    const hookPointCounts: Record<string, number> = {};
    hooks.forEach(h => {
        h.hook_points.forEach(p => {
            hookPointCounts[p] = (hookPointCounts[p] || 0) + 1;
        });
    });

    return (
        <motion.div
            initial={{ opacity: 0, y: 10 }}
            animate={{ opacity: 1, y: 0 }}
            className="flex-1 flex flex-col h-full overflow-hidden"
        >
            <div className="p-8 pb-4 space-y-6 flex-none max-w-5xl w-full mx-auto">
                <div className="flex items-center justify-between gap-4 flex-wrap">
                    <div>
                        <h1 className="text-3xl font-bold tracking-tight">Lifecycle Hooks</h1>
                        <p className="text-muted-foreground mt-1">
                            Interceptors that run at agent lifecycle points — filter, transform, or reject events.
                        </p>
                    </div>

                    <div className="flex items-center gap-3">
                        <div className="px-4 py-2 rounded-xl bg-primary/10 border border-primary/20 text-primary flex items-center gap-2 text-sm font-bold shadow-lg shadow-primary/5">
                            <Anchor className="w-4 h-4" />
                            {hooks.length} registered
                        </div>
                        <button
                            onClick={() => {
                                setIsLoading(true);
                                fetchHooks();
                            }}
                            className="p-2.5 rounded-xl bg-card border border-white/10 hover:bg-white/5 transition-colors shadow-sm"
                        >
                            <RefreshCw className={cn("w-4 h-4", isLoading && "animate-spin")} />
                        </button>
                    </div>
                </div>

                {/* Hook point summary */}
                {Object.keys(hookPointCounts).length > 0 && (
                    <div className="flex flex-wrap gap-2">
                        {Object.entries(hookPointCounts)
                            .sort(([, a], [, b]) => b - a)
                            .map(([point, count]) => (
                                <div
                                    key={point}
                                    className={cn(
                                        "inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium border",
                                        HOOK_POINT_COLORS[point] || 'bg-white/5 text-muted-foreground border-white/10'
                                    )}
                                >
                                    {HOOK_POINT_ICONS[point]}
                                    {point}
                                    <span className="font-bold ml-0.5">× {count}</span>
                                </div>
                            ))}
                    </div>
                )}
            </div>

            <div className="flex-1 overflow-y-auto px-8 pb-8 scrollbar-hide">
                <div className="max-w-5xl mx-auto space-y-3">
                    {isLoading && hooks.length === 0 ? (
                        <div className="space-y-3">
                            {[1, 2, 3].map(i => (
                                <div key={i} className="h-24 rounded-2xl border border-white/5 bg-white/[0.02] animate-pulse" />
                            ))}
                        </div>
                    ) : hooks.length > 0 ? (
                        <AnimatePresence mode="popLayout">
                            {hooks.map(hook => (
                                <HookCard key={hook.name} hook={hook} />
                            ))}
                        </AnimatePresence>
                    ) : (
                        <div className="py-20 flex flex-col items-center justify-center text-center space-y-4">
                            <div className="p-4 rounded-full bg-white/5 border border-white/10">
                                <Anchor className="w-8 h-8 text-muted-foreground" />
                            </div>
                            <div>
                                <h3 className="text-lg font-semibold">No hooks registered</h3>
                                <p className="text-sm text-muted-foreground mt-1">
                                    Hooks are registered at startup from workspace, plugin, and bundled hook configurations.
                                </p>
                            </div>
                        </div>
                    )}

                    {/* Info section */}
                    <div className="mt-8 p-6 rounded-2xl border bg-primary/5 border-primary/10 flex gap-4">
                        <div className="p-2 bg-primary/10 rounded-xl h-fit">
                            <AlertCircle className="w-5 h-5 text-primary" />
                        </div>
                        <div>
                            <h4 className="text-sm font-semibold text-primary uppercase tracking-wider">Hook Lifecycle</h4>
                            <p className="text-sm text-muted-foreground mt-1 leading-relaxed">
                                Hooks execute in priority order (lower number = higher priority). A hook can pass through,
                                modify content, or reject the event entirely. <strong>Fail-Open</strong> hooks continue processing on error,
                                while <strong>Fail-Closed</strong> hooks block the pipeline.
                            </p>
                        </div>
                    </div>
                </div>
            </div>
        </motion.div>
    );
}
