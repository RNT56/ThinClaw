import { useState } from 'react';
import { AnimatePresence, motion } from 'framer-motion';
import { Anchor, ArrowUpDown, ChevronDown, Clock, Shield, Trash2 } from 'lucide-react';

import { cn } from '../../../lib/utils';
import type * as thinclaw from '../../../lib/thinclaw';
import { HOOK_POINT_ICONS, HOOK_POINT_STYLE } from './templates';

export function HookCard({ hook, onRemove }: { hook: thinclaw.HookInfoItem; onRemove: () => void }) {
    const [expanded, setExpanded] = useState(false);
    const isFailClosed = hook.failure_mode === 'FailClosed';
    const isBuiltin = hook.name.startsWith('builtin.');

    return (
        <motion.div
            layout
            className={cn(
                "rounded-2xl border transition-all duration-300",
                "bg-white/2 border-white/5 hover:border-border/40",
                "shadow-xs hover:shadow-md"
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
                                        HOOK_POINT_STYLE
                                    )}
                                >
                                    {HOOK_POINT_ICONS[point]}
                                    {point}
                                </span>
                            ))}
                        </div>
                    </div>
                </div>
                <div className="flex items-center gap-2 flex-none mt-1">
                    {!isBuiltin && (
                        <button
                            onClick={(e) => { e.stopPropagation(); onRemove(); }}
                            className="p-1.5 rounded-lg hover:bg-red-500/10 text-muted-foreground hover:text-red-400 transition-colors"
                            title="Remove hook"
                        >
                            <Trash2 className="w-3.5 h-3.5" />
                        </button>
                    )}
                    <ChevronDown className={cn(
                        "w-4 h-4 text-muted-foreground transition-transform",
                        expanded && "rotate-180"
                    )} />
                </div>
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
                                <div className="p-3 rounded-lg bg-white/3 border border-white/5">
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
                                <div className="p-3 rounded-lg bg-white/3 border border-white/5">
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
                                <div className="p-3 rounded-lg bg-white/3 border border-white/5">
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
                                <div className="p-3 rounded-lg bg-white/3 border border-white/5">
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
